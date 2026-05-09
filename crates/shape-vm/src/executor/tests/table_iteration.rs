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
use shape_value::datatable::{DataTable, DataTableBuilder};
use shape_value::heap_value::HeapKind;
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
/// Phase-2c surface: this helper builds a `Constant::Value(ValueWord)` to
/// inject the table into bytecode. The `Constant::Value` variant carries
/// the deleted `ValueWord` carrier; rebuilding it as a kinded constant
/// (raw bits + `NativeKind::Ptr(HeapKind::DataTable)`) requires extending
/// the bytecode `Constant` enum with a kinded variant. That extension is
/// out-of-scope for E-tests sub-cluster (territory: 3 test files); see
/// ADR-006 §2.7.4 (host-tier API rebuild) and the `Constant::Value`
/// downstream cascade pinned for Phase 2c.
#[allow(dead_code)]
fn run_table_count_loop(_table_arc: Arc<DataTable>) -> i64 {
    todo!("phase-2c — see ADR-006 §2.7.4 (Constant::Value(ValueWord) carrier deleted; kinded constant variant pending)")
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
    // Phase-2c surface: TypedTable(schema_id, table) packed-encoding lived
    // inside the deleted ValueWord; no kinded equivalent exists at the
    // test boundary. See ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (TypedTable carrier pending kinded redesign)");
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

#[test]
fn test_iter_done_datatable_false_when_in_bounds() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    // Push table and idx=0, call IterDone
    push_datatable(&mut vm, table);
    push_int(&mut vm, 0);
    vm.op_iter_done().unwrap();
    assert!(
        !pop_bool(&mut vm),
        "idx=0 with 3 rows should not be done"
    );
}

#[test]
fn test_iter_done_datatable_true_at_end() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, 3);
    vm.op_iter_done().unwrap();
    assert!(
        pop_bool(&mut vm),
        "idx=3 with 3 rows should be done"
    );
}

#[test]
fn test_iter_done_typed_table_boundary() {
    // Phase-2c surface: TypedTable(schema_id, table) packed-encoding lived
    // inside the deleted ValueWord; no kinded equivalent exists at the
    // test boundary. See ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (TypedTable carrier pending kinded redesign)");
}

#[test]
fn test_iter_done_negative_index() {
    let table = make_sample_table();
    let mut vm = VirtualMachine::new(VMConfig::default());
    push_datatable(&mut vm, table);
    push_int(&mut vm, -1);
    vm.op_iter_done().unwrap();
    assert!(
        pop_bool(&mut vm),
        "negative index should be treated as done"
    );
}

#[test]
fn test_iter_next_datatable_returns_row_view() {
    // Phase-2c surface: `IterNext` on a DataTable produces a RowView whose
    // (schema_id, table, row_idx) tuple lived inside the deleted
    // ValueWord packed-tag encoding. The kinded redesign of RowView is
    // out-of-scope for the E-tests sub-cluster. See ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (RowView carrier pending kinded redesign)");
}

#[test]
fn test_iter_next_typed_table_preserves_schema_id() {
    // Phase-2c surface: TypedTable + RowView packed encodings — see
    // ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (TypedTable + RowView carrier pending kinded redesign)");
}

#[test]
fn test_iter_next_out_of_bounds_returns_none() {
    // Phase-2c surface: `is_none()` on an `IterNext` result depends on the
    // deleted ValueWord null-tag encoding. The kinded equivalent reads
    // the popped `(bits, kind)` and matches on a kinded null sentinel,
    // but the producing opcode (`IterNext` on a DataTable) currently
    // emits the legacy ValueWord null-tag form. See ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (IterNext null-result carrier pending kinded redesign)");
}

#[test]
fn test_iter_next_negative_index_returns_none() {
    // Phase-2c surface: same as test_iter_next_out_of_bounds_returns_none.
    todo!("phase-2c — see ADR-006 §2.7.4 (IterNext null-result carrier pending kinded redesign)");
}

#[test]
fn test_iter_next_all_rows_sequential() {
    // Phase-2c surface: RowView packed encoding — see
    // ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (RowView carrier pending kinded redesign)");
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
    // Phase-2c surface: RowView packed encoding — see ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (RowView carrier pending kinded redesign)");
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
    // Phase-2c surface: TypedTable packed encoding — see ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (TypedTable carrier pending kinded redesign)");
}
