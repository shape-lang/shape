// Phase-1.B Wave-β surface: this module contained heavy test-tier
// consumers of deleted host-tier carriers (the dynamic value carrier,
// the dynamic constant variant, the rare-heap-data variant, the legacy
// extension test-function registrar). Per playbook §7 REVISED part 4 +
// ADR-006 §2.7.4 (host-tier eval/marshal API rebuild), test bodies
// bound to deleted types are surfaced as `todo!()` until Phase-2c
// restores the kinded host-tier carriers (kinded constant variant,
// kinded marshal layer, test-function registrar rebuild on the new
// (NativeKind, u64) slot projection).
//
// Helpers `execute_bytecode` / `execute_bytecode_typed` migrated to the
// kinded `execute_raw` boundary — they return `Result<u64, VMError>`
// over raw native bits at top-of-stack. Callers inspect bits + the
// program's declared `top_level_frame.return_kind` directly (per the
// E-tests reference template `executor/v2_stack_tests.rs` and
// playbook §3 canonical rewrite).

use super::*;
use crate::bytecode::*;
use shape_value::VMError;

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
// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback, 2026-05-12):
// source-level smoke tests for `&mut self` method writeback semantics.
mod mutation_writeback;
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
// Wave 3 W17-trait-object-thunks (ADR-006 §2.7.24 Q25.C, 2026-05-12):
// per-variant smoke tests for the `op_dyn_method_call` dispatcher's
// `SelfArg` / `Generic` / `Compound` / nested-`BoxedReturn` / `Closure`
// variants. Each test pins a row of the §Q25.C.5 `VTableEntry` table.
mod trait_object_thunks;

// REMOVED: These helpers and their imports were removed during refactoring
// TODO: Re-implement these tests once the new context API is finalized
// fn create_test_market_data() -> MarketData { ... }
// fn setup_backtest_context(row_index: usize) -> ExecutionContext { ... }

/// Helper to create and execute a simple bytecode program. Returns the
/// **raw u64 bits** at the top of stack (ADR-006 §2.7.7 — host-tier
/// reads bits directly, no `ValueWord` synthesis). Pair with the
/// program's declared `top_level_frame.return_kind` (use
/// [`execute_bytecode_typed`]) to interpret the bits.
#[allow(dead_code)]
fn execute_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<u64, VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute_raw(None)
}

/// Helper to create and execute a bytecode program that declares a
/// typed top-level return kind. Returns the **raw u64 bits** at the top
/// of stack; the caller decodes against the declared `return_kind`
/// (e.g. `bits as i64`, `f64::from_bits(bits)`, `bits != 0` for bool).
/// Replaces the deleted `ValueWord` synthesis path.
#[allow(dead_code)]
fn execute_bytecode_typed(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    return_kind: crate::type_tracking::NativeKind,
) -> Result<u64, VMError> {
    use crate::type_tracking::FrameDescriptor;
    let mut frame = FrameDescriptor::new();
    frame.return_kind = Some(return_kind);
    let program = BytecodeProgram {
        instructions,
        constants,
        top_level_frame: Some(frame),
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute_raw(None)
}

#[test]
fn test_basic_arithmetic() {
    // Test: 2 + 3 = 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Push 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // Push 3
        Instruction::simple(OpCode::AddNumber),                             // Add
    ];
    let constants = vec![Constant::Number(2.0), Constant::Number(3.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 5.0);
}

#[test]
fn test_subtraction() {
    // Test: 10 - 4 = 6
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubNumber),
    ];
    let constants = vec![Constant::Number(10.0), Constant::Number(4.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 6.0);
}

#[test]
fn test_multiplication() {
    // Test: 3 * 4 = 12
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulNumber),
    ];
    let constants = vec![Constant::Number(3.0), Constant::Number(4.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 12.0);
}

#[test]
fn test_division() {
    // Test: 15 / 3 = 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::DivNumber),
    ];
    let constants = vec![Constant::Number(15.0), Constant::Number(3.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 5.0);
}

/// Regression: integer overflow must promote to f64, not silently wrap.
/// This prevents silent corruption in financial calculations.
#[test]
fn test_integer_overflow_promotes_to_f64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_integer_mul_overflow_promotes_to_f64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
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
    let val = f64::from_bits(result);
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

    // After Wave-E+5, AddInt success path pushes raw native i64 bits.
    // Stamp Int64 so the host synthesizer re-tags the bits.
    let result = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Int64,
    )
    .unwrap();
    // Should stay as integer (accessible as i64)
    assert_eq!(Some(result as i64), Some(300));
}

#[test]
fn test_comparisons() {
    // After Wave-E+5, GtNumber pushes raw native bool bits; stamp Bool
    // so the host boundary decodes via `to_bool()`.
    let bool_kind = crate::type_tracking::NativeKind::Bool;
    // Test: 5 > 3 = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GtNumber),
    ];
    let constants = vec![Constant::Number(5.0), Constant::Number(3.0)];

    let result = execute_bytecode_typed(instructions, constants, bool_kind).unwrap();
    assert_eq!(Some(result != 0), Some(true));

    // Test: 3 > 5 = false
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::GtNumber),
    ];
    let constants2 = vec![Constant::Number(5.0), Constant::Number(3.0)];

    let result2 = execute_bytecode_typed(instructions2, constants2, bool_kind).unwrap();
    assert_eq!(Some(result2 != 0), Some(false));
}

#[test]
fn test_logical_and() {
    // After Wave-E+5, And pushes raw native bool bits; stamp Bool.
    let bool_kind = crate::type_tracking::NativeKind::Bool;
    // Test: true && true = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::And),
    ];
    let constants = vec![Constant::Bool(true)];

    let result = execute_bytecode_typed(instructions, constants, bool_kind).unwrap();
    assert_eq!(Some(result != 0), Some(true));

    // Test: true && false = false
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::And),
    ];
    let constants2 = vec![Constant::Bool(true), Constant::Bool(false)];

    let result2 = execute_bytecode_typed(instructions2, constants2, bool_kind).unwrap();
    assert_eq!(Some(result2 != 0), Some(false));
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
    assert_eq!(f64::from_bits(result), 10.0);
}

#[test]
fn test_arrays() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — `to_array_arc()` accessor pending kinded heap dispatch on KindedSlot)")
}

#[test]
fn test_array_indexing() {
    // Test: [10, 20, 30][1] = 20
    //
    // Index uses `Constant::Int(1)` not `Number(1.0)`. After Wave-E+5,
    // PushConst for Number constants pushes raw native f64 bits; the
    // array indexer's untagged-bits path interprets those as i64 (per
    // op_get_prop's `Some(raw as i64)` branch), so f64 1.0 = bits
    // 0x3FF0000000000000 ≈ 4.6e18 — way out of bounds, returns None.
    // Int constants push native i64 directly, matching the indexer.
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
        Constant::Int(1),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 20.0);
}

#[test]
fn test_stack_operations() {
    // Test Dup: Push 5, Dup, Add should equal 10
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Dup),
        Instruction::simple(OpCode::AddNumber),
    ];
    let constants = vec![Constant::Number(5.0)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(f64::from_bits(result), 10.0);
}

#[test]
fn test_null_value() {
    let instructions = vec![Instruction::simple(OpCode::PushNull)];
    let constants = vec![];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert!((result == 0));
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
        Instruction::simple(OpCode::LtNumber),
        Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(5))), // Jump to index 10 (skip body)
        // Body: i = i + 1
        Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::AddNumber),
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
        result.slot().as_f64(),
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
        Instruction::simple(OpCode::GtNumber),
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
        result.clone().as_f64().unwrap(),
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
            Instruction::simple(OpCode::GteNumber),
        ],
        constants: vec![Constant::Number(5.0)],
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
        Some(true),
        "5 >= 5"
    );

    // Lte
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::LteNumber),
        ],
        constants: vec![Constant::Number(3.0), Constant::Number(5.0)],
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
        Some(true),
        "3 <= 5"
    );

    // Eq
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::EqNumber),
        ],
        constants: vec![Constant::Number(7.0)],
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
        Some(true),
        "7 == 7"
    );

    // Neq
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::NeqNumber),
        ],
        constants: vec![Constant::Number(5.0), Constant::Number(3.0)],
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
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
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
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
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
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
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
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
        top_level_frame: Some({
            let mut f = crate::type_tracking::FrameDescriptor::new();
            f.return_kind = Some(crate::type_tracking::NativeKind::Bool);
            f
        }),
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_bool(),
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
            Instruction::simple(OpCode::ModNumber),
        ],
        constants: vec![Constant::Number(10.0), Constant::Number(3.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_f64().unwrap(),
        1.0,
        "10 % 3"
    );

    // Pow
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::PowNumber),
        ],
        constants: vec![Constant::Number(2.0), Constant::Number(3.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_f64().unwrap(),
        8.0,
        "2 ^ 3"
    );

    // NegNumber (was Neg — generic Neg removed in Stage 4.2)
    vm.load_program(BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::NegNumber),
        ],
        constants: vec![Constant::Number(5.0)],
        ..Default::default()
    });
    assert_eq!(
        vm.execute(None).unwrap().clone().as_f64().unwrap(),
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
        vm.execute(None).unwrap().clone().as_f64().unwrap(),
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
        vm.execute(None).unwrap().clone().as_f64().unwrap(),
        10.0,
        "Object access"
    );
}

// ===== Type Annotation Wrapping Tests =====

#[test]
fn test_wrap_type_annotation_opcode() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_wrap_type_annotation_with_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_type_annotated_value_in_variable() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_type_annotated_value_type_name() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_type_annotated_value_to_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
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
            Instruction::simple(OpCode::AddNumber), // Should unwrap automatically for operations
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

// ===== Typed Column Access Tests =====

#[test]
fn test_load_col_f64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_i64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_str() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_bind_schema_success() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_bind_schema_missing_column() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

// ===== End-to-End Load() → BindSchema Pipeline Tests =====
//
// Phase-2c surface: helpers `make_test_pipeline_table` and
// `build_bind_schema_program` consumed the deleted `ValueWord` carrier
// (return type / param type). Removed pending host-tier kinded
// constant-table API (`Constant::Kinded { bits: u64, kind: NativeKind }`
// or similar). All call sites in the dependent test bodies are stubbed
// to `todo!()` per playbook §7 REVISED part 4.

#[test]
fn test_load_pipeline_correct_mapping() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_pipeline_f64_field_on_string_column() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_load_pipeline_string_field_on_number_column() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_load_pipeline_missing_column() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_load_pipeline_subset_columns() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_pipeline_column_alias() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_pipeline_wrong_alias() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_load_pipeline_timestamp_field() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_pipeline_numeric_promotion() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_pipeline_non_table_value() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

// ===== LoadCol* Opcode Coverage Tests =====

#[test]
fn test_load_col_bool() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_f64_from_float32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_f64_from_int64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_i64_from_int32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_str_row1() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_out_of_bounds_row() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_load_col_out_of_bounds_col() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_extension_intrinsic_takes_priority_over_ufcs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_extension_intrinsic_fallback_to_ufcs_when_no_match() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

// Phase-2c surface: helpers `compile_and_run` and
// `compile_and_run_capture_output` returned the deleted `ValueWord`
// carrier. Their many test callers (`test_hoisted_field_in_typed_object`
// and friends) call deleted methods (`.as_f64()`, `.as_i64()` on
// `ValueWord`) on the result; both helpers and callers need the
// host-tier kinded eval API rebuild. Surfaced per playbook §7 REVISED
// part 4.

#[test]
fn test_hoisted_field_in_typed_object() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_hoisted_field_stays_typed_object() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_array_index_assignment_accepts_int_keys() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "Wave B made array literals emit v2 typed opcodes unconditionally; v2 TypedArray uses refcounting (no Arc) so copy-on-write aliasing semantics differ from v1 VMArray. Test exercises v1 semantics; needs rewrite for v2 semantics."]
fn test_array_index_assignment_preserves_copy_on_write_aliasing() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_to_string_uses_display_impl() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_universal_type_method_returns_type_name() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_type_method_to_string_returns_canonical_name() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_print_uses_named_display_impl_with_using_selector() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_print_named_display_impl_supports_dollar_formatted_json_strings() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_print_supports_hash_formatted_strings() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_print_without_default_display_impl_reports_ambiguity_for_named_impls() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
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
        f64::from_bits(result.unwrap()),
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
        f64::from_bits(result.unwrap()),
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
    let n = f64::from_bits(_result_val);
    assert_eq!(n, 2.0, "count([5,10]) = 2");
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
        f64::from_bits(result.unwrap()),
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
        f64::from_bits(result2.unwrap()),
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
        f64::from_bits(result.unwrap()),
        0.0,
        "lag with no history returns default"
    );
}

#[test]
fn test_cte_compiles_and_runs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
fn test_module_context_can_invoke_shape_callable() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

// ============================================================================
// R5.4D: pin the full dispatch chain for the three new intrinsics
//
// These tests hand-build a bytecode program that pushes two operands plus
// the arg count, calls `BuiltinCall` with the new `BuiltinFunction` variant,
// and executes via the real VM. A failure here indicates a break in the
// opcode → helpers.rs → executor → kernel wiring — exactly the chain
// R5.4E depends on when it starts emitting these opcodes.
// ============================================================================

// Phase-2c surface: helpers `r5_4d_int_array` and `r5_4d_nested_matrix`
// returned the deleted dynamic-value carrier (built via array/scalar
// constructors that no longer exist). Removed pending kinded
// constant-table API. Their three test callers stubbed to `todo!()`
// per playbook §7 REVISED part 4.

#[test]
fn test_r5_4d_intrinsic_vec_add_i64_bytecode_dispatch() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_r5_4d_intrinsic_mat_add_bytecode_dispatch() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}

#[test]
fn test_r5_4d_intrinsic_mat_sub_bytecode_dispatch() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)")
}
