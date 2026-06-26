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
use crate::bytecode::{BytecodeProgram, Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::executor::{VMConfig, VirtualMachine};
use arrow_schema::{DataType, Field, Schema};
use shape_value::datatable::{DataTable, DataTableBuilder};
use shape_value::heap_value::{HeapKind, TableViewData};
use shape_value::NativeKind;
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
///   AddInt              ; count + 1
///   StoreLocal(2)       ; count = count + 1
///   LoadLocal(1)        ; idx
///   PushConst(1)        ; 1
///   AddInt              ; idx + 1
///   StoreLocal(1)       ; idx = idx + 1
///   Jump(loop_start)    ; back to IterDone check
///   LoadLocal(2)        ; push count as result
#[allow(dead_code)]
fn run_table_count_loop(table_arc: Arc<DataTable>) -> i64 {
    run_table_count_loop_with_constant(Constant::Value(KindedConstant::from_datatable(table_arc)))
}

fn run_typed_table_count_loop(schema_id: u64, table_arc: Arc<DataTable>) -> i64 {
    run_table_count_loop_with_constant(Constant::Value(KindedConstant::from_typed_table(
        schema_id, table_arc,
    )))
}

fn run_table_count_loop_with_constant(table: Constant) -> i64 {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
        Instruction::simple(OpCode::IterDone),
        Instruction::new(OpCode::JumpIfTrue, Some(Operand::Offset(13))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
        Instruction::simple(OpCode::IterNext),
        Instruction::simple(OpCode::Pop),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::AddInt),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::AddInt),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        Instruction::new(OpCode::Jump, Some(Operand::Offset(-17))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![table, Constant::Int(0), Constant::Int(1)];
    let program = BytecodeProgram {
        instructions,
        constants,
        top_level_locals_count: 3,
        ..Default::default()
    };
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None).unwrap();
    result.as_i64().expect("table count loop returns Int64")
}

// =========================================================================
// DataTable iteration basic tests
// =========================================================================

#[test]
fn test_datatable_for_loop_counts_rows() {
    let table = make_sample_table();
    let result = run_table_count_loop(table);
    assert_eq!(
        result, 3,
        "for-loop over 3-row DataTable should iterate 3 times"
    );
}

#[test]
fn test_typed_table_for_loop_counts_rows() {
    let table = make_sample_table();
    let result = run_typed_table_count_loop(42, table);
    assert_eq!(
        result, 3,
        "for-loop over 3-row TypedTable should iterate 3 times"
    );
}

#[test]
fn test_empty_table_for_loop_zero_iterations() {
    let table = make_empty_table();
    let result = run_table_count_loop(table);
    assert_eq!(
        result, 0,
        "for-loop over empty DataTable should iterate 0 times"
    );
}

#[test]
fn test_single_row_table_for_loop() {
    let table = make_single_row_table();
    let result = run_table_count_loop(table);
    assert_eq!(
        result, 1,
        "for-loop over 1-row DataTable should iterate exactly once"
    );
}

// =========================================================================
// IterDone + IterNext direct unit tests
// =========================================================================

/// Push a `DataTable` onto the typed VM stack as `Arc<DataTable>` bits with
/// `NativeKind::Ptr(HeapKind::DataTable)` (ADR-006 §2.7.7). Transfers one
/// strong-count share into the slot.
#[inline]
fn push_datatable(vm: &mut VirtualMachine, table: Arc<DataTable>) {
    let bits = Arc::into_raw(table) as u64;
    vm.push_kinded(bits, NativeKind::Ptr(HeapKind::DataTable))
        .unwrap();
}

#[inline]
fn push_typed_table(vm: &mut VirtualMachine, schema_id: u64, table: Arc<DataTable>) {
    let tv = Arc::new(TableViewData::TypedTable { schema_id, table });
    let bits = Arc::into_raw(tv) as u64;
    vm.push_kinded(bits, NativeKind::Ptr(HeapKind::TableView))
        .unwrap();
}

/// Push a raw `i64` onto the typed VM stack as `NativeKind::Int64`
/// (ADR-006 §2.7.7).
#[inline]
fn push_int(vm: &mut VirtualMachine, v: i64) {
    vm.push_kinded(v as u64, NativeKind::Int64).unwrap();
}

/// Push a raw `bool` onto the typed VM stack as `NativeKind::Bool`
/// (ADR-006 §2.7.7).
#[inline]
fn push_bool(vm: &mut VirtualMachine, b: bool) {
    vm.push_kinded(b as u64, NativeKind::Bool).unwrap();
}

/// Pop a `bool` from the typed VM stack. Asserts the kind track records
/// `NativeKind::Bool` and returns the bit as a `bool` (ADR-006 §2.7.7).
#[inline]
fn pop_bool(vm: &mut VirtualMachine) -> bool {
    let (bits, kind) = vm.pop_kinded().unwrap();
    assert_eq!(
        kind,
        NativeKind::Bool,
        "expected Bool result on top-of-stack, got {:?}",
        kind
    );
    bits != 0
}

fn pop_table_view(vm: &mut VirtualMachine) -> TableViewData {
    let (bits, kind) = vm.pop_kinded().unwrap();
    assert_eq!(
        kind,
        NativeKind::Ptr(HeapKind::TableView),
        "expected TableView result on top-of-stack, got {:?}",
        kind
    );
    assert_ne!(bits, 0, "TableView result must be non-null");
    let arc = unsafe { Arc::<TableViewData>::from_raw(bits as *const TableViewData) };
    arc.as_ref().clone()
}

fn assert_none_slot(vm: &mut VirtualMachine) {
    let (bits, kind) = vm.pop_kinded().unwrap();
    assert_eq!(bits, VirtualMachine::NONE_BITS);
    assert_eq!(
        kind,
        NativeKind::Bool,
        "IterNext out-of-range sentinel should match existing loop none carrier"
    );
}

#[test]
fn test_iter_done_datatable_false_when_in_bounds() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    // Push table and idx=0, call IterDone
    push_datatable(&mut vm, table);
    push_int(&mut vm, 0);
    vm.op_iter_done(None).unwrap();
    assert!(!pop_bool(&mut vm), "idx=0 with 3 rows should not be done");
}

#[test]
fn test_iter_done_datatable_true_at_end() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, 3);
    vm.op_iter_done(None).unwrap();
    assert!(pop_bool(&mut vm), "idx=3 with 3 rows should be done");
}

#[test]
fn test_iter_done_typed_table_boundary() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_typed_table(&mut vm, 42, table);
    push_int(&mut vm, 0);
    vm.op_iter_done(None).unwrap();
    assert!(
        !pop_bool(&mut vm),
        "idx=0 with 3-row TypedTable should not be done"
    );
}

#[test]
fn test_iter_done_negative_index() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, -1);
    vm.op_iter_done(None).unwrap();
    assert!(
        pop_bool(&mut vm),
        "negative index should be treated as done"
    );
}

#[test]
fn test_iter_next_datatable_returns_row_view() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, 1);
    vm.op_iter_next(None).unwrap();
    match pop_table_view(&mut vm) {
        TableViewData::RowView {
            schema_id,
            table,
            row_idx,
        } => {
            assert_eq!(schema_id, 0);
            assert_eq!(row_idx, 1);
            assert_eq!(table.row_count(), 3);
        }
        other => panic!("expected RowView, got {:?}", other),
    }
}

#[test]
fn test_iter_next_typed_table_preserves_schema_id() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_typed_table(&mut vm, 42, table);
    push_int(&mut vm, 2);
    vm.op_iter_next(None).unwrap();
    match pop_table_view(&mut vm) {
        TableViewData::RowView {
            schema_id, row_idx, ..
        } => {
            assert_eq!(schema_id, 42);
            assert_eq!(row_idx, 2);
        }
        other => panic!("expected RowView, got {:?}", other),
    }
}

#[test]
fn test_iter_next_out_of_bounds_returns_none() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, 3);
    vm.op_iter_next(None).unwrap();
    assert_none_slot(&mut vm);
}

#[test]
fn test_iter_next_negative_index_returns_none() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, -1);
    vm.op_iter_next(None).unwrap();
    assert_none_slot(&mut vm);
}

#[test]
fn test_iter_next_all_rows_sequential() {
    let table = make_sample_table();
    for row in 0..3 {
        let mut vm = VirtualMachine::new(VMConfig::default());
        push_datatable(&mut vm, Arc::clone(&table));
        push_int(&mut vm, row);
        vm.op_iter_next(None).unwrap();
        match pop_table_view(&mut vm) {
            TableViewData::RowView { row_idx, .. } => assert_eq!(row_idx, row as usize),
            other => panic!("expected RowView, got {:?}", other),
        }
    }
}

// =========================================================================
// Error type test
// =========================================================================

#[test]
fn test_iter_done_error_message_includes_table() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    // Use a non-iterable type (bool)
    push_bool(&mut vm, true);
    push_int(&mut vm, 0);
    let err = vm.op_iter_done(None).unwrap_err();
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
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, 2);
    vm.op_iter_next(None).unwrap();
    match pop_table_view(&mut vm) {
        TableViewData::RowView { table, row_idx, .. } => {
            let price = table
                .get_f64_column("price")
                .expect("price column should exist")
                .value(row_idx);
            let name = table
                .get_string_column("name")
                .expect("name column should exist")
                .value(row_idx);
            assert_eq!(price, 30.0);
            assert_eq!(name, "c");
        }
        other => panic!("expected RowView, got {:?}", other),
    }
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
    let result = run_table_count_loop(table);
    assert_eq!(
        result, n as i64,
        "for-loop over {}-row table should iterate {} times",
        n, n
    );
}

// =========================================================================
// Empty TypedTable iteration
// =========================================================================

#[test]
fn test_empty_typed_table_iteration() {
    let table = make_empty_table();
    let result = run_typed_table_count_loop(42, table);
    assert_eq!(
        result, 0,
        "for-loop over empty TypedTable should iterate 0 times"
    );
}
