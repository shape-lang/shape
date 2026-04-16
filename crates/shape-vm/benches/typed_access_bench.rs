//! Benchmarks comparing typed (LoadCol*) vs dynamic (GetProp) column access
//!
//! Tests the performance difference between compile-time resolved column access
//! (LoadColF64/LoadColStr) and runtime dynamic property access (GetProp).

use arrow_array::{Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use shape_value::DataTable;
use shape_value::ValueWordExt;
use shape_vm::bytecode::{Constant, Operand};
use shape_vm::{BytecodeProgram, Instruction, OpCode, VMConfig, VirtualMachine};
use std::sync::Arc;

const NUM_ROWS: usize = 10_000;

/// Create a 10,000-row table with f64 "price" and string "symbol" columns
fn create_test_table() -> Arc<DataTable> {
    let prices: Vec<f64> = (0..NUM_ROWS).map(|i| 100.0 + (i as f64) * 0.01).collect();
    let symbols: Vec<&str> = (0..NUM_ROWS)
        .map(|i| match i % 3 {
            0 => "AAPL",
            1 => "GOOG",
            _ => "MSFT",
        })
        .collect();

    let schema = Arc::new(Schema::new(vec![
        Field::new("price", DataType::Float64, false),
        Field::new("symbol", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(prices)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
    .unwrap();
    Arc::new(DataTable::new(batch))
}

/// Build a program that reads a single column from a single row via LoadCol*
fn build_typed_program(
    table: &Arc<DataTable>,
    row_idx: usize,
    opcode: OpCode,
    col_id: u32,
) -> BytecodeProgram {
    let row_view = shape_value::ValueWord::from_row_view(0, table.clone(), row_idx);
    BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(opcode, Some(Operand::ColumnAccess { col_id })),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![Constant::Value(row_view)],
        ..Default::default()
    }
}

/// Build a program that reads a property from a DataTable row via GetProp
fn build_dynamic_program(
    table: &Arc<DataTable>,
    row_idx: usize,
    prop_name: &str,
) -> BytecodeProgram {
    let row_view = shape_value::ValueWord::from_row_view(0, table.clone(), row_idx);
    BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::GetProp),
            Instruction::simple(OpCode::Halt),
        ],
        constants: vec![
            Constant::Value(row_view),
            Constant::String(prop_name.to_string()),
        ],
        ..Default::default()
    }
}

fn execute_program(program: &BytecodeProgram) {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program.clone());
    let _ = black_box(vm.execute(None));
}

fn benchmark_typed_vs_dynamic(c: &mut Criterion) {
    let table = create_test_table();

    let mut group = c.benchmark_group("typed_vs_dynamic_access");

    // Benchmark typed f64 access (LoadColF64)
    group.bench_function("load_col_f64", |b| {
        let programs: Vec<_> = (0..100)
            .map(|i| build_typed_program(&table, i * 100, OpCode::LoadColF64, 0))
            .collect();
        b.iter(|| {
            for program in &programs {
                execute_program(black_box(program));
            }
        });
    });

    // Benchmark dynamic f64 access (GetProp)
    group.bench_function("get_prop_f64", |b| {
        let programs: Vec<_> = (0..100)
            .map(|i| build_dynamic_program(&table, i * 100, "price"))
            .collect();
        b.iter(|| {
            for program in &programs {
                execute_program(black_box(program));
            }
        });
    });

    // Benchmark typed string access (LoadColStr)
    group.bench_function("load_col_str", |b| {
        let programs: Vec<_> = (0..100)
            .map(|i| build_typed_program(&table, i * 100, OpCode::LoadColStr, 1))
            .collect();
        b.iter(|| {
            for program in &programs {
                execute_program(black_box(program));
            }
        });
    });

    // Benchmark dynamic string access (GetProp)
    group.bench_function("get_prop_str", |b| {
        let programs: Vec<_> = (0..100)
            .map(|i| build_dynamic_program(&table, i * 100, "symbol"))
            .collect();
        b.iter(|| {
            for program in &programs {
                execute_program(black_box(program));
            }
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_typed_vs_dynamic);
criterion_main!(benches);
