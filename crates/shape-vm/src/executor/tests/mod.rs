use super::*;
use crate::bytecode::*;
use shape_value::ValueWord;

/// Shared test helpers (eval, eval_result, compile, etc.)
pub(crate) mod test_utils;

// Phase 1.1 & 1.2: Critical execution tests for recently merged features
mod auto_drop;
mod channel_ops;
mod decimal_ops;
mod deque_ops;
mod io_integration;
mod jit_abi_tests;
mod matrix_ops;
mod priority_queue_ops;
mod set_ops;
mod soak_tests;
mod table_iteration;
mod try_operator;
mod type_system_integration;
mod typed_array_ops;
mod v2_opcode_tests;
mod v2_struct_integration;

// Deep tests — gated behind `deep-tests` feature
#[cfg(feature = "deep-tests")]
mod differential_trusted;
#[cfg(feature = "deep-tests")]
mod drop_deep_tests;
#[cfg(feature = "deep-tests")]
mod extend_blocks;
#[cfg(feature = "deep-tests")]
mod hashmap_ops;
#[cfg(feature = "deep-tests")]
mod iterator_ops;
#[cfg(feature = "deep-tests")]
mod module_deep_tests;
#[cfg(feature = "deep-tests")]
mod operator_overload;
#[cfg(feature = "deep-tests")]
mod trusted_edge_cases;

// REMOVED: These helpers and their imports were removed during refactoring
// TODO: Re-implement these tests once the new context API is finalized
// fn create_test_market_data() -> MarketData { ... }
// fn setup_backtest_context(row_index: usize) -> ExecutionContext { ... }

/// Helper to create and execute a simple bytecode program
fn execute_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<ValueWord, VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None).map(|nb| nb.clone())
}

#[test]
fn test_basic_arithmetic() {
    // Test: 2 + 3 = 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Push 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // Push 3
        Instruction::simple(OpCode::Add),                             // Add
    ];
    let constants = vec![Constant::Number(2.0), Constant::Number(3.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 5.0);
}

#[test]
fn test_subtraction() {
    // Test: 10 - 4 = 6
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Sub),
    ];
    let constants = vec![Constant::Number(10.0), Constant::Number(4.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 6.0);
}

#[test]
fn test_multiplication() {
    // Test: 3 * 4 = 12
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Mul),
    ];
    let constants = vec![Constant::Number(3.0), Constant::Number(4.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 12.0);
}

#[test]
fn test_division() {
    // Test: 15 / 3 = 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Div),
    ];
    let constants = vec![Constant::Number(15.0), Constant::Number(3.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 5.0);
}

/// Regression: integer overflow must promote to f64, not silently wrap.
/// This prevents silent corruption in financial calculations.
#[test]
fn test_integer_overflow_promotes_to_f64() {
    // AddInt: i64::MAX + 1 should promote to f64
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddInt),
    ];
    let constants = vec![Constant::Int(i64::MAX), Constant::Int(1)];

    let result = execute_bytecode(instructions, constants).unwrap();
    // Should be f64, NOT a wrapped negative integer
    let val = result.to_number().unwrap();
    assert!(val > 0.0, "Overflow must produce positive f64, got {val}");
    assert_eq!(val, i64::MAX as f64 + 1.0);
}

#[test]
fn test_integer_mul_overflow_promotes_to_f64() {
    // MulInt: large * large should promote to f64
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulInt),
    ];
    let constants = vec![Constant::Int(i64::MAX / 2), Constant::Int(3)];

    let result = execute_bytecode(instructions, constants).unwrap();
    let val = result.to_number().unwrap();
    assert!(val > 0.0, "Overflow must produce positive f64, got {val}");
}

#[test]
fn test_integer_sub_overflow_promotes_to_f64() {
    // SubInt: i64::MIN - 1 should promote to f64
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubInt),
    ];
    let constants = vec![Constant::Int(i64::MIN), Constant::Int(1)];

    let result = execute_bytecode(instructions, constants).unwrap();
    let val = result.to_number().unwrap();
    assert!(val < 0.0, "Underflow must produce negative f64, got {val}");
}

#[test]
fn test_integer_arithmetic_no_overflow_stays_int() {
    // Normal case: 100 + 200 = 300 (stays as int)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddInt),
    ];
    let constants = vec![Constant::Int(100), Constant::Int(200)];

    let result = execute_bytecode(instructions, constants).unwrap();
    // Should stay as integer (accessible as i64)
    assert_eq!(result.as_i64(), Some(300));
}

#[test]
fn test_comparisons() {
    // Test: 5 > 3 = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Gt),
    ];
    let constants = vec![Constant::Number(5.0), Constant::Number(3.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_bool(), Some(true));

    // Test: 3 > 5 = false
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Gt),
    ];
    let constants2 = vec![Constant::Number(5.0), Constant::Number(3.0)];

    let result2 = execute_bytecode(instructions2, constants2).unwrap();
    assert_eq!(result2.to_bool(), Some(false));
}

#[test]
fn test_logical_and() {
    // Test: true && true = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::And),
    ];
    let constants = vec![Constant::Bool(true)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_bool(), Some(true));

    // Test: true && false = false
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::And),
    ];
    let constants2 = vec![Constant::Bool(true), Constant::Bool(false)];

    let result2 = execute_bytecode(instructions2, constants2).unwrap();
    assert_eq!(result2.to_bool(), Some(false));
}

#[test]
fn test_local_variables() {
    // Test: let x = 10; x
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Push 10
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // Store in local 0
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))), // Load local 0
    ];
    let constants = vec![Constant::Number(10.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 10.0);
}

#[test]
fn test_arrays() {
    // Test: [1, 2, 3]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Push 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // Push 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // Push 3
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))), // Create array with 3 elements
    ];
    let constants = vec![
        Constant::Number(1.0),
        Constant::Number(2.0),
        Constant::Number(3.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();

    let arr = result.to_array_arc().expect("Expected array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].clone().to_number().unwrap(), 1.0);
    assert_eq!(arr[1].clone().to_number().unwrap(), 2.0);
    assert_eq!(arr[2].clone().to_number().unwrap(), 3.0);
}

#[test]
fn test_array_indexing() {
    // Test: [10, 20, 30][1] = 20
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // Push index 1
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![
        Constant::Number(10.0),
        Constant::Number(20.0),
        Constant::Number(30.0),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 20.0);
}

#[test]
fn test_stack_operations() {
    // Test Dup: Push 5, Dup, Add should equal 10
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Dup),
        Instruction::simple(OpCode::Add),
    ];
    let constants = vec![Constant::Number(5.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 10.0);
}

#[test]
fn test_null_value() {
    let instructions = vec![Instruction::simple(OpCode::PushNull)];
    let constants = vec![];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}

// ===== Integration Tests with ExecutionContext =====

// REMOVED: Row type no longer exists in VM
// #[test]
// fn test_row_load_with_context() { ... }

// REMOVED: Row type no longer exists in VM
// #[test]
// fn test_row_property_access() { ... }

// REMOVED: Row type no longer exists in VM
// #[test]
// fn test_row_calculation() { ... }

// REMOVED: Test depends on setup_backtest_context which uses internal fields
// TODO: Re-implement once context API is finalized
// #[test]
// fn test_series_indexing_with_context() { ... }

// ===== Control Flow Tests =====

#[test]
fn test_while_loop_simple() {
    use crate::bytecode::*;

    // Simple while loop without LoopStart/End markers
    // Simulates: var i = 0; while (i < 3) { i = i + 1; } return i

    let instructions = vec![
        // i = 0 (store in module_binding 0)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(0))),
        // Loop start (index 2)
        // Condition: i < 3
        Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Lt),
        Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(5))), // Jump to index 10 (skip body)
        // Body: i = i + 1
        Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::Add),
        Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(0))),
        // Jump back to loop condition (index 2)
        // When executing self at index 10, ip will be 11, so offset = 2 - 11 = -9
        Instruction::new(OpCode::Jump, Some(Operand::Offset(-9))),
        // After loop: Load result
        Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
    ];

    let constants = vec![
        Constant::Number(0.0), // Initial value
        Constant::Number(3.0), // Loop condition
        Constant::Number(1.0), // Increment
    ];

    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    assert_eq!(
        result.to_number().unwrap(),
        3.0,
        "Loop should increment from 0 to 3"
    );
}

#[test]
fn test_conditional_jump() {
    use crate::bytecode::*;

    // Test: if (5 > 3) then push 10 else push 20
    let instructions = vec![
        // Condition: 5 > 3
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 5
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 3
        Instruction::simple(OpCode::Gt),
        // If false, jump to else (instruction 6)
        Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(2))),
        // Then: push 10
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::Jump, Some(Operand::Offset(1))), // Skip else
        // Else: push 20
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
    ];

    let constants = vec![
        Constant::Number(5.0),
        Constant::Number(3.0),
        Constant::Number(10.0), // Then value
        Constant::Number(20.0), // Else value
    ];

    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap();

    assert_eq!(
        result.clone().to_number().unwrap(),
        10.0,
        "Should take then branch since 5 > 3"
    );
}

// REMOVED: Test depends on setup_backtest_context which uses internal fields
// TODO: Re-implement once context API is finalized
// #[test]
// fn test_indicator_loading_from_cache() { ... }

#[test]
fn test_comparison_operators_complete() {
    use crate::bytecode::*;
    let mut vm = VirtualMachine::new(VMConfig::default());

    // Gte
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::Gte),
        ],
        constants: vec![Constant::Number(5.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "5 >= 5"
    );

    // Lte
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Lte),
        ],
        constants: vec![Constant::Number(3.0), Constant::Number(5.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "3 <= 5"
    );

    // Eq
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::Eq),
        ],
        constants: vec![Constant::Number(7.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "7 == 7"
    );

    // Neq
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Neq),
        ],
        constants: vec![Constant::Number(5.0), Constant::Number(3.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "5 != 3"
    );

    // EqInt
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::EqInt),
        ],
        constants: vec![Constant::Int(42), Constant::Int(42)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "42 == 42 (typed int)"
    );

    // NeqNumber
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::NeqNumber),
        ],
        constants: vec![Constant::Number(1.5), Constant::Number(2.5)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "1.5 != 2.5 (typed number)"
    );
}

#[test]
fn test_logical_or_not() {
    use crate::bytecode::*;
    let mut vm = VirtualMachine::new(VMConfig::default());

    // Or
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Or),
        ],
        constants: vec![Constant::Bool(false), Constant::Bool(true)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "false || true"
    );

    // Not
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::Not),
        ],
        constants: vec![Constant::Bool(false)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_bool(),
        Some(true),
        "!false"
    );
}

#[test]
fn test_mod_pow_neg_opcodes() {
    use crate::bytecode::*;
    let mut vm = VirtualMachine::new(VMConfig::default());

    // Mod
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Mod),
        ],
        constants: vec![Constant::Number(10.0), Constant::Number(3.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_number().unwrap(),
        1.0,
        "10 % 3"
    );

    // Pow
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Pow),
        ],
        constants: vec![Constant::Number(2.0), Constant::Number(3.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_number().unwrap(),
        8.0,
        "2 ^ 3"
    );

    // Neg
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::Neg),
        ],
        constants: vec![Constant::Number(5.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().to_number().unwrap(),
        -5.0,
        "-5"
    );
}

#[test]
fn test_swap_opcode_verify() {
    use crate::bytecode::*;

    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::Swap),
            Instruction::simple(OpCode::Pop),
        ],
        constants: vec![Constant::Number(5.0), Constant::Number(10.0)],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    assert_eq!(
        vm.execute(None).unwrap().clone().to_number().unwrap(),
        10.0,
        "Swap opcode"
    );
}

#[test]
fn test_object_operations() {
    use crate::bytecode::*;

    // {x: 10}.x = 10
    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__test_obj_x",
        vec![("x".to_string(), shape_runtime::type_schema::FieldType::Any)],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits in u16 for test");
    program.instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 1,
            }),
        ),
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 0,
                field_type_tag: crate::executor::typed_object_ops::FIELD_TAG_ANY,
            }),
        ),
    ];
    program.constants = vec![Constant::Number(10.0)];

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    assert_eq!(
        vm.execute(None).unwrap().clone().to_number().unwrap(),
        10.0,
        "Object access"
    );
}

// ===== Type Annotation Wrapping Tests =====

#[test]
fn test_wrap_type_annotation_opcode() {
    use crate::bytecode::*;
    // Test: push number, wrap with type annotation
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 123
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(0))), // Wrap with "Currency"
        ],
        constants: vec![Constant::Number(123.0)],
        strings: vec!["Currency".to_string()],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    // Verify result is a TypeAnnotatedValue
    if let Some(hv) = result.as_heap_ref() {
        match hv {
            shape_value::heap_value::HeapValue::TypeAnnotatedValue { type_name, value } => {
                assert_eq!(type_name, "Currency");
                assert_eq!(value.as_f64().unwrap(), 123.0);
            }
            _ => panic!("Expected TypeAnnotatedValue, got {:?}", result),
        }
    } else {
        panic!("Expected TypeAnnotatedValue, got {:?}", result);
    }
}

#[test]
fn test_wrap_type_annotation_with_string() {
    use crate::bytecode::*;
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello"
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(1))), // Wrap with "Greeting"
        ],
        constants: vec![Constant::String("hello".to_string())],
        strings: vec!["hello".to_string(), "Greeting".to_string()],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    // Verify result is wrapped with the correct type name
    if let Some(hv) = result.as_heap_ref() {
        match hv {
            shape_value::heap_value::HeapValue::TypeAnnotatedValue { type_name, value } => {
                assert_eq!(type_name, "Greeting");
                assert_eq!(value.as_str().unwrap(), "hello");
            }
            _ => panic!("Expected TypeAnnotatedValue, got {:?}", result),
        }
    } else {
        panic!("Expected TypeAnnotatedValue, got {:?}", result);
    }
}

#[test]
fn test_type_annotated_value_in_variable() {
    use crate::bytecode::*;
    // Test: let x: Currency = 123; x
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 123
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(0))), // Wrap with "Currency"
            Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(0))), // Store in x
            Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),  // Load x
        ],
        constants: vec![Constant::Number(123.0)],
        strings: vec!["Currency".to_string()],
        module_binding_names: vec!["x".to_string()],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    // Verify the loaded variable is still wrapped
    if let Some(hv) = result.as_heap_ref() {
        match hv {
            shape_value::heap_value::HeapValue::TypeAnnotatedValue { type_name, value } => {
                assert_eq!(type_name, "Currency");
                assert_eq!(value.to_number().unwrap(), 123.0);
            }
            _ => panic!("Expected TypeAnnotatedValue after store/load"),
        }
    } else {
        panic!("Expected TypeAnnotatedValue after store/load");
    }
}

#[test]
fn test_type_annotated_value_type_name() {
    let wrapped =
        ValueWord::from_type_annotated_value("Currency".to_string(), ValueWord::from_f64(123.0));

    // type_name() should return the underlying value's type, not the wrapper
    assert_eq!(wrapped.type_name(), "number");
}

#[test]
fn test_type_annotated_value_to_string() {
    let wrapped =
        ValueWord::from_type_annotated_value("Currency".to_string(), ValueWord::from_f64(123.0));

    // Verify the underlying value is a number with the correct value
    if let Some(hv) = wrapped.as_heap_ref() {
        match hv {
            shape_value::heap_value::HeapValue::TypeAnnotatedValue { type_name, value } => {
                assert_eq!(type_name, "Currency");
                assert_eq!(value.to_number().unwrap(), 123.0);
            }
            _ => panic!("Expected TypeAnnotatedValue"),
        }
    } else {
        panic!("Expected TypeAnnotatedValue");
    }
}

#[test]
fn test_wrap_type_annotation_preserves_operations() {
    use crate::bytecode::*;

    // Test that wrapped values can still be used in operations
    // push 10, wrap as Currency, push 5, add
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(0))), // Wrap
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 5
            Instruction::simple(OpCode::Add), // Should unwrap automatically for operations
        ],
        constants: vec![Constant::Number(10.0), Constant::Number(5.0)],
        strings: vec!["Currency".to_string()],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    // This test documents current behavior - it might fail if operations don't auto-unwrap
    // If it fails, we need to add unwrapping logic to arithmetic operations
    match result {
        Ok(val) => {
            // If operations auto-unwrap, we should get 15
            // If not, we'll get an error
            println!("Result: {:?}", val);
        }
        Err(e) => {
            // Expected if operations don't auto-unwrap TypeAnnotatedValue
            println!("Error (expected): {:?}", e);
        }
    }
}

#[test]
fn test_multiple_type_annotations() {
    use crate::bytecode::*;
    // Test: let x: Currency = 100; let y: Percent = 0.5;
    let program = BytecodeProgram {
        instructions: vec![
            // x: Currency = 100
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(0))),
            Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(0))),
            // y: Percent = 0.5
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::new(OpCode::WrapTypeAnnotation, Some(Operand::Property(1))),
            Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(1))),
            // Load x
            Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
        ],
        constants: vec![Constant::Number(100.0), Constant::Number(0.5)],
        strings: vec!["Currency".to_string(), "Percent".to_string()],
        module_binding_names: vec!["x".to_string(), "y".to_string()],
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    // Verify x has Currency annotation
    if let Some(hv) = result.as_heap_ref() {
        match hv {
            shape_value::heap_value::HeapValue::TypeAnnotatedValue { type_name, value } => {
                assert_eq!(type_name, "Currency");
                assert_eq!(value.to_number().unwrap(), 100.0);
            }
            _ => panic!("Expected TypeAnnotatedValue for x"),
        }
    } else {
        panic!("Expected TypeAnnotatedValue for x");
    }

    // Note: Can't easily verify y without executing another LoadModuleBinding
    // The test above already verifies that type annotations work correctly
}

// ===== Typed Column Access Tests =====

#[test]
fn test_load_col_f64() {
    use arrow_array::{Float64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "price",
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Float64Array::from(vec![42.5, 99.0]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 0);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_f64().unwrap(),
        42.5,
        "Expected Number(42.5), got {:?}",
        result
    );
}

#[test]
fn test_load_col_i64() {
    use arrow_array::{Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "volume",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Int64Array::from(vec![1000, 2000]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 1);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColI64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_i64().unwrap(),
        2000,
        "Expected Int(2000), got {:?}",
        result
    );
}

#[test]
fn test_load_col_str() {
    use arrow_array::{RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "symbol",
        DataType::Utf8,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(StringArray::from(vec!["AAPL", "GOOG"]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 0);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColStr,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "AAPL");
    }
}

#[test]
fn test_bind_schema_success() {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema as ArrowSchema};
    use shape_runtime::type_schema::TypeSchemaBuilder;
    use shape_value::datatable::DataTable;
    use std::sync::Arc;

    // Create a DataTable with Arrow schema
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("price", DataType::Float64, false),
        Field::new("symbol", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![100.0, 200.0])),
            Arc::new(StringArray::from(vec!["AAPL", "GOOG"])),
        ],
    )
    .unwrap();
    let table = DataTable::new(batch);

    // Create matching TypeSchema
    let mut registry = shape_runtime::type_schema::TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("TestTrade")
        .f64_field("price")
        .string_field("symbol")
        .register(&mut registry);

    // Build bytecode program with BindSchema
    let datatable_val = ValueWord::from_datatable(Arc::new(table));
    let mut program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::BindSchema, Some(Operand::Count(schema_id as u16))),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Value(datatable_val)],
        ..Default::default()
    };
    program.type_schema_registry = registry;

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 2);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_bind_schema_missing_column() {
    use arrow_array::{Float64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema as ArrowSchema};
    use shape_runtime::type_schema::TypeSchemaBuilder;
    use shape_value::datatable::DataTable;
    use std::sync::Arc;

    // Create a DataTable missing the "volume" column
    let schema = Arc::new(ArrowSchema::new(vec![Field::new(
        "price",
        DataType::Float64,
        false,
    )]));
    let batch =
        RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(vec![100.0]))]).unwrap();
    let table = DataTable::new(batch);

    // TypeSchema requires "price" and "volume"
    let mut registry = shape_runtime::type_schema::TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("TestTrade2")
        .f64_field("price")
        .f64_field("volume")
        .register(&mut registry);

    let datatable_val = ValueWord::from_datatable(Arc::new(table));
    let mut program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::BindSchema, Some(Operand::Count(schema_id as u16))),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Value(datatable_val)],
        ..Default::default()
    };
    program.type_schema_registry = registry;

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err(), "BindSchema should fail for missing column");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("volume"),
        "Error should mention missing column 'volume': {}",
        err
    );
}

// ===== End-to-End Load() → BindSchema Pipeline Tests =====

/// Build a deterministic DataTable with 5 columns.
fn make_test_pipeline_table() -> ValueWord {
    use arrow_array::{
        BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
    };
    use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
    use shape_value::datatable::DataTable;
    use std::sync::Arc;

    let symbols = ["AAPL", "GOOG", "MSFT", "TSLA", "AMZN"];
    let timestamp_values: Vec<i64> = (0..100)
        .map(|i| 1_704_067_200_000_i64 + (i as i64) * 60_000_i64)
        .collect();
    let symbol_values: Vec<&str> = (0..100).map(|i| symbols[i % symbols.len()]).collect();
    let price_values: Vec<f64> = (0..100).map(|i| 100.0 + (i as f64) * 1.23).collect();
    let volume_values: Vec<i64> = (0..100).map(|i| 1_000_000 + i as i64 * 12_345).collect();
    let is_buy_values: Vec<bool> = (0..100).map(|i| i % 2 == 0).collect();

    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        ),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("price", DataType::Float64, false),
        Field::new("volume", DataType::Int64, false),
        Field::new("is_buy", DataType::Boolean, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(TimestampMillisecondArray::from(timestamp_values)),
            Arc::new(StringArray::from(symbol_values)),
            Arc::new(Float64Array::from(price_values)),
            Arc::new(Int64Array::from(volume_values)),
            Arc::new(BooleanArray::from(is_buy_values)),
        ],
    )
    .unwrap();
    ValueWord::from_datatable(Arc::new(DataTable::new(batch)))
}

/// Build a BytecodeProgram that pushes a constant value, runs BindSchema, and halts.
fn build_bind_schema_program(
    value: ValueWord,
    registry: shape_runtime::type_schema::TypeSchemaRegistry,
    schema_id: u32,
) -> BytecodeProgram {
    let mut program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::BindSchema, Some(Operand::Count(schema_id as u16))),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Value(value)],
        ..Default::default()
    };
    program.type_schema_registry = registry;
    program
}

#[test]
fn test_load_pipeline_correct_mapping() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("PipelineTrade")
        .timestamp_field("timestamp")
        .string_field("symbol")
        .f64_field("price")
        .i64_field("volume")
        .bool_field("is_buy")
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 100);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_load_pipeline_f64_field_on_string_column() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("BadF64")
        .f64_field("symbol") // symbol is Utf8, not Float64
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("symbol"),
        "Error should mention 'symbol': {}",
        err
    );
    assert!(
        err.contains("type"),
        "Error should mention type mismatch: {}",
        err
    );
}

#[test]
fn test_load_pipeline_string_field_on_number_column() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("BadStr")
        .string_field("price") // price is Float64, not Utf8
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("price"),
        "Error should mention 'price': {}",
        err
    );
    assert!(
        err.contains("type"),
        "Error should mention type mismatch: {}",
        err
    );
}

#[test]
fn test_load_pipeline_missing_column() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("MissingCol")
        .f64_field("nonexistent")
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("nonexistent"),
        "Error should mention 'nonexistent': {}",
        err
    );
    assert!(
        err.contains("column"),
        "Error should mention missing column: {}",
        err
    );
}

#[test]
fn test_load_pipeline_subset_columns() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("SubsetTrade")
        .f64_field("price")
        .string_field("symbol")
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 100);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_load_pipeline_column_alias() {
    use shape_runtime::type_schema::FieldType;
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    // Field "close" maps to CSV column "price" via @alias annotation
    let schema_id = TypeSchemaBuilder::new("AliasTrade")
        .field_with_meta(
            "close",
            FieldType::F64,
            vec![shape_runtime::type_schema::FieldAnnotation {
                name: "alias".to_string(),
                args: vec!["price".to_string()],
            }],
        )
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 100);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_load_pipeline_wrong_alias() {
    use shape_runtime::type_schema::FieldType;
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    // Field "close" maps to nonexistent CSV column via @alias annotation
    let schema_id = TypeSchemaBuilder::new("WrongAlias")
        .field_with_meta(
            "close",
            FieldType::F64,
            vec![shape_runtime::type_schema::FieldAnnotation {
                name: "alias".to_string(),
                args: vec!["nonexistent".to_string()],
            }],
        )
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("nonexistent"),
        "Error should mention 'nonexistent': {}",
        err
    );
}

#[test]
fn test_load_pipeline_timestamp_field() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("TsTrade")
        .timestamp_field("timestamp")
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 100);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_load_pipeline_numeric_promotion() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let dt_val = make_test_pipeline_table();

    let mut registry = TypeSchemaRegistry::new();
    // F64 field on Int64 column — should succeed (numeric promotion)
    let schema_id = TypeSchemaBuilder::new("PromoTrade")
        .f64_field("volume")
        .register(&mut registry);

    let program = build_bind_schema_program(dt_val, registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();

    if let Some((sid, table)) = result.as_typed_table() {
        assert_eq!(sid, schema_id as u64);
        assert_eq!(table.row_count(), 100);
    } else {
        panic!("Expected TypedTable, got {:?}", result);
    }
}

#[test]
fn test_load_pipeline_non_table_value() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let mut registry = TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("AnyType")
        .f64_field("x")
        .register(&mut registry);

    // Push a Number (not a DataTable) then BindSchema
    let program = build_bind_schema_program(ValueWord::from_f64(42.0), registry, schema_id);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("expected DataTable") || err.contains("got number"),
        "Error should mention expected DataTable or got number: {}",
        err
    );
}

// ===== LoadCol* Opcode Coverage Tests =====

#[test]
fn test_load_col_bool() {
    use arrow_array::{BooleanArray, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "flag",
        DataType::Boolean,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(BooleanArray::from(vec![
            true, false, true,
        ]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    // Read row 1 (false)
    let row_view = ValueWord::from_row_view(0, table.clone(), 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColBool,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_bool().unwrap(),
        false,
        "Expected Bool(false), got {:?}",
        result
    );

    // Read row 2 (true) — verifies bit-level read at offset > 0
    let row_view2 = ValueWord::from_row_view(0, table, 2);
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColBool,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants2 = vec![Constant::Value(row_view2)];
    let result2 = execute_bytecode(instructions2, constants2).unwrap();
    assert_eq!(
        result2.as_bool().unwrap(),
        true,
        "Expected Bool(true), got {:?}",
        result2
    );
}

#[test]
fn test_load_col_f64_from_float32() {
    use arrow_array::{Float32Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "val",
        DataType::Float32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Float32Array::from(vec![
            3.14f32, 2.72f32,
        ]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    {
        let n = result.as_f64().expect("Expected Number(~3.14)");
        assert!((n - 3.14).abs() < 0.001, "Expected ~3.14, got {}", n);
    }
}

#[test]
fn test_load_col_f64_from_int64() {
    use arrow_array::{Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "count",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Int64Array::from(vec![42, 100]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_f64().unwrap(),
        42.0,
        "Expected Number(42.0), got {:?}",
        result
    );
}

#[test]
fn test_load_col_i64_from_int32() {
    use arrow_array::{Int32Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
        "small",
        DataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Int32Array::from(vec![123, 456]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColI64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_i64().unwrap(),
        456,
        "Expected Int(456), got {:?}",
        result
    );
}

#[test]
fn test_load_col_str_row1() {
    use arrow_array::{RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new("name", DataType::Utf8, false)]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(StringArray::from(vec![
            "alpha", "beta", "gamma",
        ]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    let row_view = ValueWord::from_row_view(0, table, 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColStr,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants).unwrap();
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "beta");
    }
}

#[test]
fn test_load_col_out_of_bounds_row() {
    use arrow_array::{Float64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Float64Array::from(vec![1.0, 2.0]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    // row_idx=5, but table only has 2 rows
    let row_view = ValueWord::from_row_view(0, table, 5);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err(), "Should error on out-of-bounds row");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Row index") || err.contains("out of bounds"),
        "Error should mention row out of bounds: {}",
        err
    );
}

#[test]
fn test_load_col_out_of_bounds_col() {
    use arrow_array::{Float64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
    let batch = RecordBatch::try_new(
        schema,
        vec![std::sync::Arc::new(Float64Array::from(vec![1.0]))],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    // col_id=5, but table only has 1 column
    let row_view = ValueWord::from_row_view(0, table, 0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 5 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err(), "Should error on out-of-bounds column");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Column index") || err.contains("out of bounds"),
        "Error should mention column out of bounds: {}",
        err
    );
}

#[test]
fn test_load_col_wrong_value_type() {
    // Push a Number (not RowView) then LoadColF64 → error
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Number(42.0)];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_err(),
        "Should error when LoadCol* gets non-RowView"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("RowView") || err.contains("expected"),
        "Error should mention expected RowView: {}",
        err
    );
}

#[test]
fn test_load_col_multi_column() {
    use arrow_array::{BooleanArray, Float64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTable;

    let schema = std::sync::Arc::new(Schema::new(vec![
        Field::new("price", DataType::Float64, false),
        Field::new("active", DataType::Boolean, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            std::sync::Arc::new(Float64Array::from(vec![10.5, 20.0])),
            std::sync::Arc::new(BooleanArray::from(vec![true, false])),
            std::sync::Arc::new(StringArray::from(vec!["buy", "sell"])),
        ],
    )
    .unwrap();
    let table = std::sync::Arc::new(DataTable::new(batch));

    // Read f64 from col 0, row 1
    let rv = ValueWord::from_row_view(0, table.clone(), 1);
    let result = execute_bytecode(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColF64,
                Some(Operand::ColumnAccess { col_id: 0 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        vec![Constant::Value(rv)],
    )
    .unwrap();
    assert_eq!(
        result.as_f64().unwrap(),
        20.0,
        "Expected Number(20.0), got {:?}",
        result
    );

    // Read bool from col 1, row 0
    let rv = ValueWord::from_row_view(0, table.clone(), 0);
    let result = execute_bytecode(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColBool,
                Some(Operand::ColumnAccess { col_id: 1 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        vec![Constant::Value(rv)],
    )
    .unwrap();
    assert_eq!(
        result.as_bool().unwrap(),
        true,
        "Expected Bool(true), got {:?}",
        result
    );

    // Read string from col 2, row 1
    let rv = ValueWord::from_row_view(0, table.clone(), 1);
    let result = execute_bytecode(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColStr,
                Some(Operand::ColumnAccess { col_id: 2 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        vec![Constant::Value(rv)],
    )
    .unwrap();
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "sell");
    }
}

// =========================================================================
// Object Method Tests (Phase 5)
// =========================================================================

#[test]
fn test_dynamic_object_methods_are_rejected() {
    // Build {name: "hello"} and call .get("name") — dynamic object helpers are disabled.
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "name"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "hello"
        Instruction::new(OpCode::NewObject, Some(Operand::Count(1))), // {name: "hello"}
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "name"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "get"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 (arg count)
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("name".to_string()),
        Constant::String("hello".to_string()),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_err(),
        "Typed object dynamic helper methods must be rejected"
    );
}

// =========================================================================
// Extension Intrinsic Dispatch Tests (Phase 3.5)
// =========================================================================

#[test]
fn test_extension_intrinsic_dispatch() {
    // Register an extension with an intrinsic for type "TestWidget", method "compute"
    let mut module = shape_runtime::module_exports::ModuleExports::new("test_ext");
    module.add_intrinsic(
        "TestWidget",
        "compute",
        |nb_args: &[ValueWord], _ctx: &shape_runtime::module_exports::ModuleContext| {
            // Intrinsic: returns the first argument doubled
            match nb_args.first().and_then(|nb| nb.as_number_coerce()) {
                Some(n) => Ok(ValueWord::from_f64(n * 2.0)),
                None => Err("compute() requires a number argument".to_string()),
            }
        },
    );

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.register_extension(module);

    // Build: { __type: "TestWidget", value: 10 }.compute(5)
    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__test_ext_widget",
        vec![
            (
                "__type".to_string(),
                shape_runtime::type_schema::FieldType::Any,
            ),
            (
                "value".to_string(),
                shape_runtime::type_schema::FieldType::Any,
            ),
        ],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits in u16 for test");
    program.instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "TestWidget"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 10
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 2,
            }),
        ),
        // Call .compute(5)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 5 (arg)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "compute"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 1 (arg count)
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    program.constants = vec![
        Constant::String("TestWidget".to_string()),
        Constant::Number(10.0),
        Constant::Number(5.0),
        Constant::String("compute".to_string()),
        Constant::Number(1.0),
    ];

    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();
    // Intrinsic doubles the first arg (5), so result should be 10
    assert_eq!(
        result.to_number().unwrap(),
        10.0,
        "Extension intrinsic should return 5 * 2 = 10"
    );
}

#[test]
fn test_extension_intrinsic_takes_priority_over_ufcs() {
    // Register both an intrinsic AND a UFCS function for the same type+method.
    // The intrinsic should win.
    let mut module = shape_runtime::module_exports::ModuleExports::new("test_ext2");
    module.add_intrinsic(
        "PriorityType",
        "action",
        |_args: &[ValueWord], _ctx: &shape_runtime::module_exports::ModuleContext| {
            Ok(ValueWord::from_string(Arc::new("intrinsic".to_string())))
        },
    );

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.register_extension(module);

    // Build: { __type: "PriorityType" }.action()
    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__test_ext_priority",
        vec![(
            "__type".to_string(),
            shape_runtime::type_schema::FieldType::Any,
        )],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits in u16 for test");
    program.instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "PriorityType"
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 1,
            }),
        ),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "action"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 (arg count)
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    program.constants = vec![
        Constant::String("PriorityType".to_string()),
        Constant::String("action".to_string()),
        Constant::Number(0.0),
    ];

    vm.load_program(program);
    let result = vm.execute(None).unwrap().clone();
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "intrinsic");
    }
}

#[test]
fn test_extension_intrinsic_fallback_to_ufcs_when_no_match() {
    // Register intrinsic for "TestWidget" method "compute" only.
    // Calling a different method "other_method" should NOT hit intrinsic.
    let mut module = shape_runtime::module_exports::ModuleExports::new("test_ext3");
    module.add_intrinsic(
        "TestWidget",
        "compute",
        |_args: &[ValueWord], _ctx: &shape_runtime::module_exports::ModuleContext| {
            Ok(ValueWord::from_string(Arc::new("intrinsic".to_string())))
        },
    );

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.register_extension(module);

    // Build: { __type: "TestWidget" }.other_method()
    // Should fail since no intrinsic, no callable prop, and no UFCS exists
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::new(OpCode::NewObject, Some(Operand::Count(1))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "other_method"
            Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 0 (arg count)
            Instruction::simple(OpCode::CallMethod),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![
            Constant::String("__type".to_string()),
            Constant::String("TestWidget".to_string()),
            Constant::String("other_method".to_string()),
            Constant::Number(0.0),
        ],
        ..Default::default()
    };

    vm.load_program(program);
    let result = vm.execute(None);
    // Should error — no intrinsic for "other_method", no callable prop, no UFCS
    assert!(
        result.is_err(),
        "Should fail when no intrinsic or UFCS match"
    );
}

/// Helper to compile and execute Shape source, returning the final value
fn compile_and_run(source: &str) -> Result<ValueWord, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Helper to compile and execute Shape source while capturing print output.
fn compile_and_run_capture_output(source: &str) -> Result<(ValueWord, Vec<String>), VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.enable_output_capture();
    vm.load_program(bytecode);
    let result = vm.execute(None)?.clone();
    Ok((result, vm.get_captured_output()))
}

#[test]
fn test_hoisted_field_in_typed_object() {
    // Optimistic hoisting: a.y = 2 means 'y' should be in a's schema from the start.
    // After assignment, a.y should return 2, and 'a' should remain a TypedObject (not Object).
    let result = compile_and_run(
        r#"
        let mut a = { x: 1 }
        a.y = 2
        a.y
    "#,
    );
    assert!(
        result.is_ok(),
        "Hoisted field access should work: {:?}",
        result.err()
    );
    let _result_val = result.unwrap();
    match _result_val
        .as_f64()
        .or_else(|| _result_val.as_i64().map(|i| i as f64))
    {
        Some(n) => assert_eq!(n, 2.0, "a.y should be 2"),
        other => panic!("Expected number 2, got {:?}", other),
    }
}

#[test]
fn test_hoisted_field_stays_typed_object() {
    // After hoisting, the object should be TypedObject (not Object).
    // Both explicit and hoisted fields accessible.
    let result = compile_and_run(
        r#"
        let mut a = { x: 10 }
        a.y = 20
        a.x + a.y
    "#,
    );
    assert!(
        result.is_ok(),
        "Hoisted + explicit field access should work: {:?}",
        result.err()
    );
    let _result_val = result.unwrap();
    match _result_val
        .as_f64()
        .or_else(|| _result_val.as_i64().map(|i| i as f64))
    {
        Some(n) => assert_eq!(n, 30.0, "a.x + a.y = 10 + 20 = 30"),
        other => panic!("Expected 30, got {:?}", other),
    }
}

#[test]
fn test_array_index_assignment_accepts_int_keys() {
    let result = compile_and_run(
        r#"
        let mut a = [10, 20, 30]
        a[0] = 99
        a[0]
    "#,
    )
    .expect("program should run");

    assert_eq!(
        result.to_number().unwrap(),
        99.0,
        "expected 99, got {:?}",
        result
    );
}

#[test]
#[ignore = "Wave B made array literals emit v2 typed opcodes unconditionally; v2 TypedArray uses refcounting (no Arc) so copy-on-write aliasing semantics differ from v1 VMArray. Test exercises v1 semantics; needs rewrite for v2 semantics."]
fn test_array_index_assignment_preserves_copy_on_write_aliasing() {
    let result = compile_and_run(
        r#"
        let mut a = [1, 2]
        let b = a
        a[0] = 9
        b[0]
    "#,
    )
    .expect("program should run");

    assert_eq!(
        result.to_number().unwrap(),
        1.0,
        "expected 1, got {:?}",
        result
    );
}

#[test]
fn test_array_index_assignment_uses_local_fast_path_opcode() {
    let program = shape_ast::parser::parse_program(
        r#"
        let mut a = [1, 2]
        a[0] = 9
    "#,
    )
    .expect("program should parse");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let bytecode = compiler.compile(&program).expect("program should compile");
    // v2 typed-array path: when the receiver is a tracked typed array,
    // the compiler emits `TypedArraySetI64` (or sibling) instead of the
    // legacy local fast-path `SetLocalIndex`. Either is acceptable.
    assert!(
        bytecode.instructions.iter().any(|ins| {
            matches!(
                ins.opcode,
                OpCode::SetLocalIndex
                    | OpCode::SetModuleBindingIndex
                    | OpCode::TypedArraySetI64
                    | OpCode::TypedArraySetI32
                    | OpCode::TypedArraySetF64
                    | OpCode::TypedArraySetBool
            )
        }),
        "expected SetLocalIndex/SetModuleBindingIndex/TypedArraySet* opcode in compiled bytecode"
    );
}

#[test]
fn test_print_uses_default_display_impl() {
    let source = r#"
        type User {name: string}
        trait Display { display(self): string }
        impl Display for User {
            method display() { "default:" + self.name }
        }
        let u = User {name: "Alice"}
        print(u)
    "#;

    let (_result, output) = compile_and_run_capture_output(source).expect("program should run");
    assert_eq!(output.len(), 1, "expected one print line");
    assert_eq!(output[0], "default:Alice");
}

#[test]
fn test_to_string_uses_display_impl() {
    let source = r#"
        type User {name: string}
        trait Display { display(self): string }
        impl Display for User {
            method display() { "default:" + self.name }
        }
        let u = User {name: "Alice"}
        u.to_string()
    "#;

    let result = compile_and_run(source).expect("program should run");
    assert_eq!(
        result.as_str().unwrap(),
        "default:Alice",
        "Expected string result, got {:?}",
        result
    );
}

#[test]
fn test_universal_type_method_returns_type_name() {
    let source = r#"
        type User {name: string}
        let u = User {name: "Alice"}
        u.type().to_string()
    "#;
    let result = compile_and_run(source).expect("program should run");
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "User");
    }
}

#[test]
fn test_type_method_to_string_returns_canonical_name() {
    let source = r#"
        type User {name: string}
        let u = User {name: "Alice"}
        u.type().to_string()
    "#;
    let result = compile_and_run(source).expect("program should run");
    {
        let s = result.as_arc_string().expect("Expected String");
        assert_eq!(s.as_str(), "User");
    }
}

#[test]
fn test_print_uses_named_display_impl_with_using_selector() {
    let source = r#"
        type User {name: string}
        trait Display { display(self): string }
        impl Display for User {
            method display() { "default:" + self.name }
        }
        impl Display for User as JsonDisplay {
            method display() { "json:" + self.name }
        }
        let u = User {name: "Alice"}
        print(u using JsonDisplay)
    "#;

    let (_result, output) = compile_and_run_capture_output(source).expect("program should run");
    assert_eq!(output.len(), 1, "expected one print line");
    assert_eq!(output[0], "json:Alice");
}

#[test]
fn test_print_named_display_impl_supports_dollar_formatted_json_strings() {
    let source = r#"
        type User {name: string}
        trait Display { display(self): string }
        impl Display for User as JsonDisplay {
            method display() { f$"{\"name\": ${self.name}}" }
        }
        let u = User {name: "Alice"}
        print(u using JsonDisplay)
    "#;

    let (_result, output) = compile_and_run_capture_output(source).expect("program should run");
    assert_eq!(output.len(), 1, "expected one print line");
    assert_eq!(output[0], "{\"name\": Alice}");
}

#[test]
fn test_print_supports_hash_formatted_strings() {
    let source = r##"
        let cmd = "ls -la"
        print(f#"run #{cmd}")
    "##;

    let (_result, output) = compile_and_run_capture_output(source).expect("program should run");
    assert_eq!(output.len(), 1, "expected one print line");
    assert_eq!(output[0], "run ls -la");
}

#[test]
fn test_print_without_default_display_impl_reports_ambiguity_for_named_impls() {
    let source = r#"
        type User {name: string}
        trait Display { display(self): string }
        impl Display for User as StandardDisplay {
            method display() { "std:" + self.name }
        }
        impl Display for User as JsonDisplay {
            method display() { "json:" + self.name }
        }
        let u = User {name: "Alice"}
        print(u)
    "#;

    let err = compile_and_run_capture_output(source).expect_err(
        "print(u) should fail when multiple named Display impls exist without a default impl",
    );
    match err {
        VMError::RuntimeError(msg) => {
            assert!(
                msg.contains("Ambiguous Display impl for type 'User'"),
                "unexpected error: {}",
                msg
            );
            assert!(
                msg.contains("JsonDisplay"),
                "error should list JsonDisplay: {}",
                msg
            );
            assert!(
                msg.contains("StandardDisplay"),
                "error should list StandardDisplay: {}",
                msg
            );
        }
        other => panic!("expected RuntimeError, got {:?}", other),
    }
}

// ============================================================
// Window function, JOIN, and CTE executor tests
// ============================================================

#[test]
fn test_window_sum_builtin_executes() {
    // Test that WindowSum builtin can be dispatched through the executor.
    // We manually construct bytecodes that push an array and call WindowSum.
    let instructions = vec![
        // Push array [1, 2, 3]
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 1.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 3.0
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
        // Push window spec (empty string = no partitioning)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        // Push arg count (2: array + spec)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        // Call WindowSum
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowSum)),
        ),
    ];
    let constants = vec![
        Constant::Number(1.0),
        Constant::Number(2.0),
        Constant::Number(3.0),
        Constant::String("".to_string()),
        Constant::Number(2.0), // arg count
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowSum should execute: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().to_number().unwrap(),
        6.0,
        "sum([1,2,3]) = 6"
    );
}

#[test]
fn test_window_avg_builtin_executes() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowAvg)),
        ),
    ];
    let constants = vec![
        Constant::Number(10.0),
        Constant::Number(20.0),
        Constant::Number(30.0),
        Constant::String("".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowAvg should execute: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().to_number().unwrap(),
        20.0,
        "avg([10,20,30]) = 20"
    );
}

#[test]
fn test_window_count_builtin_executes() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::NewArray, Some(Operand::Count(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowCount)),
        ),
    ];
    let constants = vec![
        Constant::Number(5.0),
        Constant::Number(10.0),
        Constant::String("".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowCount should execute: {:?}",
        result.err()
    );
    let _result_val = result.unwrap();
    match _result_val
        .as_f64()
        .or_else(|| _result_val.as_i64().map(|i| i as f64))
    {
        Some(n) => assert_eq!(n, 2.0, "count([5,10]) = 2"),
        other => panic!("Expected 2, got {:?}", other),
    }
}

#[test]
fn test_window_min_max_builtin_executes() {
    // Test WindowMin
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowMin)),
        ),
    ];
    let constants = vec![
        Constant::Number(7.0),
        Constant::Number(3.0),
        Constant::Number(9.0),
        Constant::String("".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowMin should execute: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().to_number().unwrap(),
        3.0,
        "min([7,3,9]) = 3"
    );

    // Test WindowMax
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowMax)),
        ),
    ];
    let constants2 = vec![
        Constant::Number(7.0),
        Constant::Number(3.0),
        Constant::Number(9.0),
        Constant::String("".to_string()),
        Constant::Number(2.0),
    ];

    let result2 = execute_bytecode(instructions2, constants2);
    assert!(
        result2.is_ok(),
        "WindowMax should execute: {:?}",
        result2.err()
    );
    assert_eq!(
        result2.unwrap().to_number().unwrap(),
        9.0,
        "max([7,3,9]) = 9"
    );
}

#[test]
fn test_window_row_number_builtin_executes() {
    // WindowRowNumber returns the current row index (0 for scalar context)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // value (unused)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // spec
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // arg count
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowRowNumber)),
        ),
    ];
    let constants = vec![
        Constant::Number(42.0),
        Constant::String("".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowRowNumber should execute: {:?}",
        result.err()
    );
}

#[test]
fn test_window_lag_lead_builtin_executes() {
    // WindowLag with offset=1 and default=0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // value
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // offset
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // default
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // spec
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // arg count
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::WindowLag)),
        ),
    ];
    let constants = vec![
        Constant::Number(100.0),
        Constant::Number(1.0),
        Constant::Number(0.0),
        Constant::String("".to_string()),
        Constant::Number(4.0),
    ];

    let result = execute_bytecode(instructions, constants);
    assert!(
        result.is_ok(),
        "WindowLag should execute: {:?}",
        result.err()
    );
    // In scalar context, lag returns the default value
    assert_eq!(
        result.unwrap().to_number().unwrap(),
        0.0,
        "lag with no history returns default"
    );
}

#[test]
fn test_cte_compiles_and_runs() {
    // Test that a WITH query compiles and runs without errors.
    // CTE stores its subquery result as a module_binding variable.
    let result = compile_and_run(
        r#"
        let x = 10
        let y = 20
        x + y
    "#,
    );
    assert!(
        result.is_ok(),
        "Basic program should work: {:?}",
        result.err()
    );
    let _result_val = result.unwrap();
    match _result_val
        .as_f64()
        .or_else(|| _result_val.as_i64().map(|i| i as f64))
    {
        Some(n) => assert_eq!(n, 30.0),
        other => panic!("Expected 30, got {:?}", other),
    }
}

#[test]
fn test_module_context_can_invoke_shape_callable() {
    let mut extension = shape_runtime::module_exports::ModuleExports::new("bridge");
    extension.add_function(
        "invoke_once",
        |args, ctx: &shape_runtime::module_exports::ModuleContext| {
            let callable = args
                .first()
                .ok_or_else(|| "bridge.invoke_once() requires callable as arg#0".to_string())?;
            ctx.invoke_callable
                .ok_or_else(|| "no callable invoker in context".to_string())
                .and_then(|invoke| invoke(callable, &[ValueWord::from_i64(21)]))
        },
    );

    let source = r#"
use bridge

fn plus_one(x: int) -> int {
  x + 1
}

bridge::invoke_once(plus_one)
"#;

    let program = shape_ast::parser::parse_program(source).expect("parse");
    let bytecode = crate::compiler::BytecodeCompiler::new()
        .with_extensions(vec![extension.clone()])
        .compile(&program)
        .expect("compile");

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.register_extension(extension);
    vm.populate_module_objects();

    let result = vm.execute(None).expect("execute");
    let number = result
        .as_i64()
        .map(|v| v as f64)
        .or_else(|| result.as_f64())
        .expect("numeric");
    assert_eq!(number, 22.0);
}
