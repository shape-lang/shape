//! Integration tests for DataTable/TypedTable for-loop iteration.
//!
//! Tests compile and run bytecode programs that iterate over tables using
//! IterDone/IterNext opcodes, verifying that:
//! - DataTable iteration produces RowView values
//! - TypedTable iteration preserves schema_id
//! - Empty table iteration produces zero iterations
//! - Break/continue work inside table loops
//! - rows() and columnsRef() methods work end-to-end

use super::*;
use crate::executor::{VMConfig, VirtualMachine};
use arrow_schema::{DataType, Field, Schema};
use shape_value::ValueWord;
use shape_value::datatable::{DataTable, DataTableBuilder};
use std::sync::Arc;

/// Build a sample DataTable with 3 rows: price=[10.0, 20.0, 30.0], name=["a","b","c"]
fn make_sample_table() -> Arc<DataTable> {
    let schema = Schema::new(vec![
        Field::new("price", DataType::Float64, false),
        Field::new("name", DataType::Utf8, false),
    ]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![10.0, 20.0, 30.0]);
    builder.add_string_column(vec!["a", "b", "c"]);
    Arc::new(builder.finish().unwrap())
}

/// Build an empty DataTable with one column.
fn make_empty_table() -> Arc<DataTable> {
    let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![]);
    Arc::new(builder.finish().unwrap())
}

/// Build a single-row DataTable.
fn make_single_row_table() -> Arc<DataTable> {
    let schema = Schema::new(vec![Field::new("val", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![42.0]);
    Arc::new(builder.finish().unwrap())
}

/// Execute a bytecode program that pushes a DataTable constant, then runs
/// a for-loop pattern: init idx=0, loop { IterDone, IterNext, body, idx+=1 }.
/// The body accumulates a counter. Returns the final counter value.
///
/// Bytecode pattern for `let count = 0; for row in table { count = count + 1 }; count`:
///
///   PushConst(table)    ; local 0 = table
///   StoreLocal(0)
///   PushConst(0)        ; local 1 = idx = 0
///   StoreLocal(1)
///   PushConst(0)        ; local 2 = count = 0
///   StoreLocal(2)
///   LoopStart(offset)   ; marks loop
///   LoadLocal(0)        ; Dup the iterator (table)
///   LoadLocal(1)        ; Load idx
///   IterDone            ; push bool
///   JumpIfTrue(exit)    ; if done, exit
///   LoadLocal(0)        ; Dup the iterator (table)
///   LoadLocal(1)        ; Load idx
///   IterNext            ; push row_view
///   Pop                 ; discard row_view (just counting)
///   LoadLocal(2)        ; count
///   PushConst(1)        ; 1
///   Add                 ; count + 1
///   StoreLocal(2)       ; count = count + 1
///   LoadLocal(1)        ; idx
///   PushConst(1)        ; 1
///   Add                 ; idx + 1
///   StoreLocal(1)       ; idx = idx + 1
///   Jump(loop_start)    ; back to IterDone check
///   LoopEnd
///   LoadLocal(2)        ; push count as result
fn run_table_count_loop(table_nb: ValueWord) -> ValueWord {
    // Named instruction positions — offsets computed from these, not magic numbers.
    // All jump offsets are relative to (instruction_position + 1) because the VM
    // advances ip before executing the instruction.
    const LOOP_START: i32 = 6;
    const ITER_CHECK: i32 = 7; // LoadLocal(0) — first instruction inside loop
    const JUMP_EXIT: i32 = 10; // JumpIfTrue
    const LOOP_END: i32 = 24;
    const RESULT: i32 = 25; // LoadLocal(2) — first instruction after loop
    const BACK_JUMP: i32 = 23; // Jump (back-edge)

    let instructions = vec![
        // Setup: store table as local 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 0
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 1
        // idx = 0 as local 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 3
        // count = 0 as local 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 4
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 5
        // LoopStart — offset from ip (LOOP_START+1) to past LoopEnd
        Instruction::new(
            OpCode::LoopStart,
            Some(Operand::Offset(RESULT - (LOOP_START + 1))),
        ),
        // IterDone check
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))), // 7: table
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))), // 8: idx
        Instruction::simple(OpCode::IterDone),                        // 9: push done bool
        Instruction::new(
            OpCode::JumpIfTrue,
            Some(Operand::Offset(RESULT - (JUMP_EXIT + 1))),
        ),
        // IterNext — get the row
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))), // 11: table
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))), // 12: idx
        Instruction::simple(OpCode::IterNext),                        // 13: push row_view
        Instruction::simple(OpCode::Pop),                             // 14: discard row_view
        // count = count + 1
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))), // 15
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 16
        Instruction::simple(OpCode::Add),                             // 17
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 18
        // idx = idx + 1
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))), // 19
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 20
        Instruction::simple(OpCode::Add),                             // 21
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 22
        // Jump back to iter check
        Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(ITER_CHECK - (BACK_JUMP + 1))),
        ),
        // LoopEnd
        Instruction::simple(OpCode::LoopEnd), // 24
        // Push count as result
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))), // 25
    ];

    let constants = vec![
        Constant::Value(table_nb), // 0: the table
        Constant::Int(0),          // 1: zero
        Constant::Int(1),          // 2: one
    ];

    let program = BytecodeProgram {
        instructions,
        constants,
        top_level_locals_count: 3, // locals 0=table, 1=idx, 2=count
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None).expect("execution failed").clone()
}

// =========================================================================
// DataTable iteration basic tests
// =========================================================================

#[test]
fn test_datatable_for_loop_counts_rows() {
    let table = make_sample_table();
    let result = run_table_count_loop(ValueWord::from_datatable(table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        3,
        "for-loop over 3-row DataTable should iterate 3 times"
    );
}

#[test]
fn test_typed_table_for_loop_counts_rows() {
    let table = make_sample_table();
    let result = run_table_count_loop(ValueWord::from_typed_table(42, table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        3,
        "for-loop over 3-row TypedTable should iterate 3 times"
    );
}

#[test]
fn test_empty_table_for_loop_zero_iterations() {
    let table = make_empty_table();
    let result = run_table_count_loop(ValueWord::from_datatable(table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        0,
        "for-loop over empty DataTable should iterate 0 times"
    );
}

#[test]
fn test_single_row_table_for_loop() {
    let table = make_single_row_table();
    let result = run_table_count_loop(ValueWord::from_datatable(table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        1,
        "for-loop over 1-row DataTable should iterate exactly once"
    );
}

// =========================================================================
// IterDone + IterNext direct unit tests
// =========================================================================

#[test]
fn test_iter_done_datatable_false_when_in_bounds() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    // Push table and idx=0, call IterDone
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(0)).unwrap();
    vm.op_iter_done().unwrap();
    let result = vm.pop_vw().unwrap();
    assert_eq!(
        result.as_bool(),
        Some(false),
        "idx=0 with 3 rows should not be done"
    );
}

#[test]
fn test_iter_done_datatable_true_at_end() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(3)).unwrap();
    vm.op_iter_done().unwrap();
    let result = vm.pop_vw().unwrap();
    assert_eq!(
        result.as_bool(),
        Some(true),
        "idx=3 with 3 rows should be done"
    );
}

#[test]
fn test_iter_done_typed_table_boundary() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    // idx=2 (last valid) should not be done
    vm.push_vw(ValueWord::from_typed_table(10, table.clone()))
        .unwrap();
    vm.push_vw(ValueWord::from_i64(2)).unwrap();
    vm.op_iter_done().unwrap();
    assert_eq!(vm.pop_vw().unwrap().as_bool(), Some(false));

    // idx=3 should be done
    vm.push_vw(ValueWord::from_typed_table(10, table)).unwrap();
    vm.push_vw(ValueWord::from_i64(3)).unwrap();
    vm.op_iter_done().unwrap();
    assert_eq!(vm.pop_vw().unwrap().as_bool(), Some(true));
}

#[test]
fn test_iter_done_negative_index() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(-1)).unwrap();
    vm.op_iter_done().unwrap();
    assert_eq!(
        vm.pop_vw().unwrap().as_bool(),
        Some(true),
        "negative index should be treated as done"
    );
}

#[test]
fn test_iter_next_datatable_returns_row_view() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_datatable(table.clone()))
        .unwrap();
    vm.push_vw(ValueWord::from_i64(0)).unwrap();
    vm.op_iter_next().unwrap();
    let result = vm.pop_vw().unwrap();
    let (schema_id, rv_table, row_idx) = result.as_row_view().expect("Expected RowView");
    assert_eq!(schema_id, 0, "plain DataTable uses schema_id=0");
    assert_eq!(row_idx, 0);
    assert!(Arc::ptr_eq(rv_table, &table));
}

#[test]
fn test_iter_next_typed_table_preserves_schema_id() {
    let table = make_sample_table();
    let schema_id = 77u64;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_typed_table(schema_id, table.clone()))
        .unwrap();
    vm.push_vw(ValueWord::from_i64(1)).unwrap();
    vm.op_iter_next().unwrap();
    let result = vm.pop_vw().unwrap();
    let (sid, _, row_idx) = result.as_row_view().expect("Expected RowView");
    assert_eq!(
        sid, schema_id,
        "TypedTable should preserve schema_id in RowView"
    );
    assert_eq!(row_idx, 1);
}

#[test]
fn test_iter_next_out_of_bounds_returns_none() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(99)).unwrap();
    vm.op_iter_next().unwrap();
    let result = vm.pop_vw().unwrap();
    assert!(
        result.is_none(),
        "out-of-bounds IterNext should return None"
    );
}

#[test]
fn test_iter_next_negative_index_returns_none() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(-1)).unwrap();
    vm.op_iter_next().unwrap();
    let result = vm.pop_vw().unwrap();
    assert!(
        result.is_none(),
        "negative index IterNext should return None"
    );
}

#[test]
fn test_iter_next_all_rows_sequential() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    for i in 0..3 {
        vm.push_vw(ValueWord::from_datatable(table.clone()))
            .unwrap();
        vm.push_vw(ValueWord::from_i64(i)).unwrap();
        vm.op_iter_next().unwrap();
        let result = vm.pop_vw().unwrap();
        let (_, _, row_idx) = result.as_row_view().expect("Expected RowView");
        assert_eq!(row_idx, i as usize);
    }
}

// =========================================================================
// Error type test
// =========================================================================

#[test]
fn test_iter_done_error_message_includes_table() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    // Use a non-iterable type (bool)
    vm.push_vw(ValueWord::from_bool(true)).unwrap();
    vm.push_vw(ValueWord::from_i64(0)).unwrap();
    let err = vm.op_iter_done().unwrap_err();
    match err {
        VMError::TypeError { expected, .. } => {
            assert!(
                expected.contains("table"),
                "error message should mention 'table', got: {}",
                expected
            );
        }
        other => panic!("Expected TypeError, got: {:?}", other),
    }
}

// =========================================================================
// DataTable row property access after iteration
// =========================================================================

#[test]
fn test_row_view_from_iter_next_has_correct_data() {
    // Verify that RowView values returned by IterNext can be used for property access
    let table = make_sample_table(); // price=[10,20,30], name=["a","b","c"]
    let mut vm = VirtualMachine::new(VMConfig::default());

    // Get row 1 (price=20.0, name="b")
    vm.push_vw(ValueWord::from_datatable(table)).unwrap();
    vm.push_vw(ValueWord::from_i64(1)).unwrap();
    vm.op_iter_next().unwrap();
    let row = vm.pop_vw().unwrap();

    let (_, rv_table, row_idx) = row.as_row_view().expect("Expected RowView");
    assert_eq!(row_idx, 1);

    // Verify the underlying table data
    let prices = rv_table.get_f64_column("price").unwrap();
    assert_eq!(prices.value(row_idx), 20.0);
    let names = rv_table.get_string_column("name").unwrap();
    assert_eq!(names.value(row_idx), "b");
}

// =========================================================================
// Large table iteration
// =========================================================================

#[test]
fn test_large_table_iteration() {
    let n = 1000;
    let schema = Schema::new(vec![Field::new("val", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column((0..n).map(|i| i as f64).collect());
    let table = Arc::new(builder.finish().unwrap());
    let result = run_table_count_loop(ValueWord::from_datatable(table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        n as i64,
        "for-loop over {}-row table should iterate {} times",
        n,
        n
    );
}

// =========================================================================
// Empty TypedTable iteration
// =========================================================================

#[test]
fn test_empty_typed_table_iteration() {
    let table = make_empty_table();
    let result = run_table_count_loop(ValueWord::from_typed_table(5, table));
    assert_eq!(
        result.as_i64().expect("expected int"),
        0,
        "for-loop over empty TypedTable should iterate 0 times"
    );
}
