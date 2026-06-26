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

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use super::*;
use crate::bytecode::*;
use shape_value::{KindedSlot, VMError};

/// Shared test helpers (eval, eval_result, compile, etc.)
pub(crate) mod test_utils;

// Phase 1.1 & 1.2: Critical execution tests for recently merged features
mod auto_drop;
mod channel_ops;
mod decimal_ops;
mod deque_ops;
mod io_integration;
mod jit_abi_tests;
// Strict-typing defection sentinel — scans the source tree for the
// Bool-default slot-fabrication pattern (ADR-006 §2.7.7 forbidden),
// mirroring scripts/check-no-dynamic.sh at the Rust-test layer.
mod matrix_ops;
// R1 named-fn-as-value carrier tests — a named function referenced as a
// value (captured into an escaping closure, forwarded as a call arg, or
// passed to an array HOF) must dispatch correctly and NEVER SIGSEGV.
mod named_fn_value;
mod no_dynamic;
// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback, 2026-05-12):
// source-level smoke tests for `&mut self` method writeback semantics.
mod mutation_writeback;
// ADR-006 §2.7.27 amendment (W17-pop-mutation, 2026-05-12): source-level
// smoke tests for the tuple-return `&mut self` ABI variant covering
// pop-shaped methods (Array.pop / Deque.popBack / popFront /
// PriorityQueue.pop / HashMap.remove).
mod hashmap_readback_kind;
mod pop_mutation;
mod priority_queue_ops;
mod seam_c_for_loop;
mod set_ops;
mod soak_tests;
mod string_method_aliases;
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

/// Helper to execute hand-built bytecode through the post-strict-typing
/// host boundary, preserving the actual top-of-stack [`KindedSlot`].
#[allow(dead_code)]
fn execute_bytecode_slot(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<KindedSlot, VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None)
}

/// [`execute_bytecode_slot`] variant for programs that use top-level locals.
#[allow(dead_code)]
fn execute_bytecode_slot_with_locals(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    num_locals: u16,
) -> Result<KindedSlot, VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        top_level_locals_count: num_locals,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None)
}

/// [`execute_bytecode_slot`] variant for hand-built bytecode that references
/// the program string pool, such as typed `CallMethod` instructions.
#[allow(dead_code)]
fn execute_bytecode_slot_with_strings(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    strings: Vec<String>,
) -> Result<KindedSlot, VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        strings,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None)
}

#[test]
fn test_basic_arithmetic() {
    // Test: 2 + 3 = 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Push 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // Push 3
        Instruction::simple(OpCode::AddNumber),                       // Add
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

/// `int` (i64) arithmetic is EXACT across the full i64 range; overflow is a
/// structured RUNTIME error (THE RULE, user 2026-06-01 / numeric-conversion
/// D3) — never a silent two's-complement wrap and never a silent f64
/// promotion. (This supersedes the 2026-05-20 wrapping ruling these tests
/// previously pinned: a silent wrap is the same hidden-data-loss class as a
/// silent narrowing cast.) Widen explicitly via `as number` / `as bigint`.
#[test]
fn test_integer_add_overflow_is_runtime_error() {
    // AddInt: i64::MAX + 1 overflows — D3 runtime error, no wrap.
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddInt),
    ];
    let constants = vec![Constant::Int(i64::MAX), Constant::Int(1)];

    let err = execute_bytecode(instructions, constants).unwrap_err();
    assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
}

/// `MulInt` overflow is a structured RUNTIME error (D3), not a wrap.
#[test]
fn test_integer_mul_overflow_is_runtime_error() {
    // MulInt: 3037000500^2 overflows i64 — D3 runtime error.
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulInt),
    ];
    let constants = vec![Constant::Int(3_037_000_500), Constant::Int(3_037_000_500)];

    let err = execute_bytecode(instructions, constants).unwrap_err();
    assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
}

/// `SubInt` underflow is a structured RUNTIME error (D3), not a wrap.
#[test]
fn test_integer_sub_overflow_is_runtime_error() {
    // SubInt: i64::MIN - 1 underflows — D3 runtime error.
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubInt),
    ];
    let constants = vec![Constant::Int(i64::MIN), Constant::Int(1)];

    let err = execute_bytecode(instructions, constants).unwrap_err();
    assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
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
    // T1-host-tier-marshal-rebuild (R8, 2026-05-23, ADR-006 §2.7.4 +
    // §2.7.6 / Q8): exercise array construction + length through the
    // language surface. The deleted `to_array_arc()` accessor is no
    // longer needed — `arr.len()` projects to Int64 directly.
    use test_utils::eval;
    let v = eval("[1, 2, 3].len()");
    assert_eq!(v.as_i64(), Some(3));
}

#[test]
fn test_array_indexing() {
    // Test: [10, 20, 30][1] = 20
    //
    // The legacy `NewArray` opcode has no element-kind proof, so this
    // hand-built bytecode uses the typed producer opcodes and then exercises
    // the generic `GetProp` index path against the v2 typed-array carrier.
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(3))),
        Instruction::simple(OpCode::Dup),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::simple(OpCode::Dup),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::simple(OpCode::Dup),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArrayPushF64),
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

// T1 NEW-CLASS-SURFACE (R8 2026-05-23): the `WrapTypeAnnotation` opcode
// pushes a `TypeAnnotatedValue`-style heap object that the deleted
// `ValueWord` carrier exposed via `to_type_annotated_arc()` /
// `type_annotation_name()` accessors. The kinded `HeapValue` variant
// for type-annotated values is itself out of scope for T1
// (V3-S5 ckpt-5 / W17-typed-module-exports territory — typed-Arc
// payload identity propagation through the wrap/unwrap pair). Tests
// marked `#[ignore]` with the class-shift surface name.
#[test]
#[ignore = "V3-S5 ckpt-5 SURFACE: typed-annotation HeapValue accessor pending; T1 class-shift surface (ADR-006 §2.7.4)"]
fn test_wrap_type_annotation_opcode() {
    todo!("V3-S5 ckpt-5 — typed-annotation HeapValue accessor pending; out of T1 scope")
}

#[test]
#[ignore = "V3-S5 ckpt-5 SURFACE: typed-annotation HeapValue accessor pending; T1 class-shift surface (ADR-006 §2.7.4)"]
fn test_wrap_type_annotation_with_string() {
    todo!("V3-S5 ckpt-5 — typed-annotation HeapValue accessor pending; out of T1 scope")
}

#[test]
#[ignore = "V3-S5 ckpt-5 SURFACE: typed-annotation HeapValue accessor pending; T1 class-shift surface (ADR-006 §2.7.4)"]
fn test_type_annotated_value_in_variable() {
    todo!("V3-S5 ckpt-5 — typed-annotation HeapValue accessor pending; out of T1 scope")
}

#[test]
#[ignore = "V3-S5 ckpt-5 SURFACE: typed-annotation HeapValue accessor pending; T1 class-shift surface (ADR-006 §2.7.4)"]
fn test_type_annotated_value_type_name() {
    todo!("V3-S5 ckpt-5 — typed-annotation HeapValue accessor pending; out of T1 scope")
}

#[test]
#[ignore = "V3-S5 ckpt-5 SURFACE: typed-annotation HeapValue accessor pending; T1 class-shift surface (ADR-006 §2.7.4)"]
fn test_type_annotated_value_to_string() {
    todo!("V3-S5 ckpt-5 — typed-annotation HeapValue accessor pending; out of T1 scope")
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
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_multiple_type_annotations() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

// ===== Typed Column Access Tests =====
//
// R8 W3 W17-typed-module-exports-followup-constant-pool (ADR-006 §2.7.4
// / §2.7.7 / Q9, 2026-05-24): the deleted `Constant::Value(ValueWord)`
// carrier is replaced by the kinded `Constant::Value(KindedConstant)`
// variant. Tests use `KindedConstant::from_row_view` /
// `from_datatable` to inject host-tier values; the constant holds one
// strong-count share, and every `op_push_const` bumps the refcount via
// `clone_with_kind`. Result inspection uses `vm.execute(None)` which
// returns `KindedSlot` directly — heap-typed results unwrap by
// matching `kind() == Ptr(HeapKind::TableView)` and recovering the
// `&TableViewData` from `raw()` bits (mirror of `exec_bind_schema`'s
// receiver borrow at `executor/window_join.rs:465`).

/// Test-only helper to recover a `(schema_id, &Arc<DataTable>)` from a
/// `KindedSlot` carrying a `TableView::TypedTable` payload. Returns
/// `None` if the slot's kind is not `Ptr(HeapKind::TableView)` or the
/// inner `TableViewData` is not the `TypedTable` variant.
///
/// SAFETY: borrows the inner `TableViewData` for the lifetime of
/// `&KindedSlot` — the slot owns one `Arc::into_raw(Arc<TableViewData>)`
/// strong-count share (kept alive until the slot's `Drop` runs), so
/// the borrow is sound. Mirror of the receiver-borrow shape in
/// `executor/window_join.rs:465` (`exec_bind_schema` /
/// `exec_load_col`).
#[allow(dead_code)]
fn typed_table_from_slot(
    slot: &shape_value::KindedSlot,
) -> Option<(u64, std::sync::Arc<shape_value::DataTable>)> {
    use shape_value::heap_value::TableViewData;
    use shape_value::{HeapKind, NativeKind};
    match slot.kind() {
        NativeKind::Ptr(HeapKind::TableView) => {
            let bits = slot.raw();
            if bits == 0 {
                return None;
            }
            // SAFETY: per the §2.7.7/Q9 producer-side stamp on every
            // `TableView`-kinded push site (e.g. exec_bind_schema), `bits`
            // are `Arc::into_raw::<TableViewData>` for the matching `T`.
            let tv = unsafe { &*(bits as *const TableViewData) };
            match tv {
                TableViewData::TypedTable { schema_id, table } => {
                    Some((*schema_id, std::sync::Arc::clone(table)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

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

    let row_view = KindedConstant::from_row_view(0, table, 0);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Float64,
    )
    .unwrap();
    let v = f64::from_bits(bits);
    assert_eq!(v, 42.5, "Expected 42.5, got {}", v);
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

    let row_view = KindedConstant::from_row_view(0, table, 1);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColI64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Int64,
    )
    .unwrap();
    assert_eq!(bits as i64, 2000, "Expected 2000, got {}", bits as i64);
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

    let row_view = KindedConstant::from_row_view(0, table, 0);

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColStr,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap();
    assert_eq!(result.as_str().expect("Expected String"), "AAPL");
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
    let datatable_val = KindedConstant::from_datatable(Arc::new(table));
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 2);
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

    let datatable_val = KindedConstant::from_datatable(Arc::new(table));
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

/// Build a deterministic DataTable with 5 columns, returned as a
/// `KindedConstant` (DataTable-kinded) ready to inject into a
/// `Constant::Value`.
fn make_test_pipeline_table() -> KindedConstant {
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
    KindedConstant::from_datatable(Arc::new(DataTable::new(batch)))
}

/// Build a `BytecodeProgram` that pushes a `Constant::Value`, runs
/// `BindSchema`, and halts.
fn build_bind_schema_program(
    value: KindedConstant,
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 100);
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 100);
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 100);
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 100);
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
    let result = vm.execute(None).unwrap();

    let (sid, table) = typed_table_from_slot(&result).expect("Expected TypedTable result");
    assert_eq!(sid, schema_id as u64);
    assert_eq!(table.row_count(), 100);
}

#[test]
fn test_load_pipeline_non_table_value() {
    use shape_runtime::type_schema::{TypeSchemaBuilder, TypeSchemaRegistry};
    let mut registry = shape_runtime::type_schema::TypeSchemaRegistry::new();
    let schema_id = TypeSchemaBuilder::new("AnyType")
        .f64_field("x")
        .register(&mut registry);
    let _ = TypeSchemaRegistry::new; // suppress unused import warning when only one path uses it

    // Push a Number (not a DataTable) then BindSchema — uses a plain
    // `Constant::Number` rather than `Constant::Value`.
    let mut program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::BindSchema, Some(Operand::Count(schema_id as u16))),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Number(42.0)],
        ..Default::default()
    };
    program.type_schema_registry = registry;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None);

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("expected DataTable") || err.contains("got") || err.contains("DataTable"),
        "Error should mention expected DataTable: {}",
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
    let row_view = KindedConstant::from_row_view(0, table.clone(), 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColBool,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];
    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Bool,
    )
    .unwrap();
    assert_eq!(bits != 0, false, "Expected false, got {}", bits != 0);

    // Read row 2 (true) — verifies bit-level read at offset > 0
    let row_view2 = KindedConstant::from_row_view(0, table, 2);
    let instructions2 = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColBool,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants2 = vec![Constant::Value(row_view2)];
    let bits2 = execute_bytecode_typed(
        instructions2,
        constants2,
        crate::type_tracking::NativeKind::Bool,
    )
    .unwrap();
    assert_eq!(bits2 != 0, true, "Expected true, got {}", bits2 != 0);
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

    let row_view = KindedConstant::from_row_view(0, table, 0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Float64,
    )
    .unwrap();
    let n = f64::from_bits(bits);
    assert!((n - 3.14).abs() < 0.001, "Expected ~3.14, got {}", n);
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

    let row_view = KindedConstant::from_row_view(0, table, 0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColF64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Float64,
    )
    .unwrap();
    let v = f64::from_bits(bits);
    assert_eq!(v, 42.0, "Expected 42.0, got {}", v);
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

    let row_view = KindedConstant::from_row_view(0, table, 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColI64,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let bits = execute_bytecode_typed(
        instructions,
        constants,
        crate::type_tracking::NativeKind::Int64,
    )
    .unwrap();
    assert_eq!(bits as i64, 456, "Expected 456, got {}", bits as i64);
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

    let row_view = KindedConstant::from_row_view(0, table, 1);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::LoadColStr,
            Some(Operand::ColumnAccess { col_id: 0 }),
        ),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Value(row_view)];

    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap();
    assert_eq!(result.as_str().expect("Expected String"), "beta");
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
    let row_view = KindedConstant::from_row_view(0, table, 5);
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
    let row_view = KindedConstant::from_row_view(0, table, 0);
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
    let rv = KindedConstant::from_row_view(0, table.clone(), 1);
    let bits = execute_bytecode_typed(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColF64,
                Some(Operand::ColumnAccess { col_id: 0 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        vec![Constant::Value(rv)],
        crate::type_tracking::NativeKind::Float64,
    )
    .unwrap();
    let v = f64::from_bits(bits);
    assert_eq!(v, 20.0, "Expected 20.0, got {}", v);

    // Read bool from col 1, row 0
    let rv = KindedConstant::from_row_view(0, table.clone(), 0);
    let bits = execute_bytecode_typed(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColBool,
                Some(Operand::ColumnAccess { col_id: 1 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        vec![Constant::Value(rv)],
        crate::type_tracking::NativeKind::Bool,
    )
    .unwrap();
    assert_eq!(bits != 0, true, "Expected true, got {}", bits != 0);

    // Read string from col 2, row 1
    let rv = KindedConstant::from_row_view(0, table.clone(), 1);
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(
                OpCode::LoadColStr,
                Some(Operand::ColumnAccess { col_id: 2 }),
            ),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Value(rv)],
        ..Default::default()
    };
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap();
    assert_eq!(result.as_str().expect("Expected String"), "sell");
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
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_extension_intrinsic_dispatch() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_extension_intrinsic_takes_priority_over_ufcs() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_extension_intrinsic_fallback_to_ufcs_when_no_match() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

// Phase-2c surface: helpers `compile_and_run` and
// `compile_and_run_capture_output` returned the deleted `ValueWord`
// carrier. Their many test callers (`test_hoisted_field_in_typed_object`
// and friends) call deleted methods (`.as_f64()`, `.as_i64()` on
// `ValueWord`) on the result; both helpers and callers need the
// host-tier kinded eval API rebuild. Surfaced per playbook §7 REVISED
// part 4.

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_hoisted_field_in_typed_object() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_hoisted_field_stays_typed_object() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
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
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_print_uses_default_display_impl() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_to_string_uses_display_impl() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_universal_type_method_returns_type_name() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_type_method_to_string_returns_canonical_name() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_print_uses_named_display_impl_with_using_selector() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_print_named_display_impl_supports_dollar_formatted_json_strings() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_print_supports_hash_formatted_strings() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
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
    assert_eq!(f64::from_bits(result.unwrap()), 6.0, "sum([1,2,3]) = 6");
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
    assert_eq!(f64::from_bits(result.unwrap()), 3.0, "min([7,3,9]) = 3");

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
    assert_eq!(f64::from_bits(result2.unwrap()), 9.0, "max([7,3,9]) = 9");
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
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_cte_compiles_and_runs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted helper)")
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_module_context_can_invoke_shape_callable() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
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
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_r5_4d_intrinsic_vec_add_i64_bytecode_dispatch() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_r5_4d_intrinsic_mat_add_bytecode_dispatch() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}

#[test]
#[ignore = "T1 class-shift surface (ADR-006 §2.7.4) — depends on deleted host-tier helpers / typed-Arc accessors not in T1 scope"]
fn test_r5_4d_intrinsic_mat_sub_bytecode_dispatch() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted host-tier carriers)"
    )
}
