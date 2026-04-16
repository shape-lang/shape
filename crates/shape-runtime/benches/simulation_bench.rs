use std::sync::Arc;

use arrow_array::Float64Array;
use arrow_schema::{DataType, Field, Schema};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use shape_runtime::simulation::{
    CorrelatedKernel, CorrelatedKernelConfig, DenseKernel, DenseKernelConfig, TableSchema, par_run,
};
use shape_value::DataTable;

fn make_table(rows: usize, name: &str) -> DataTable {
    let data: Vec<f64> = (0..rows).map(|i| i as f64).collect();
    let array = Arc::new(Float64Array::from(data)) as arrow_array::ArrayRef;
    let schema = Schema::new(vec![Field::new(name, DataType::Float64, false)]);
    let batch = arrow_array::RecordBatch::try_new(Arc::new(schema), vec![array]).unwrap();
    DataTable::new(batch)
}

fn bench_dense_kernel_throughput(c: &mut Criterion) {
    let table = make_table(100_000, "close");
    let kernel = DenseKernel::new(DenseKernelConfig::full(table.row_count()));

    c.bench_function("dense_kernel_throughput", |b| {
        b.iter(|| {
            let result = kernel
                .run(&table, 0.0f64, |idx, ptrs, state| {
                    unsafe {
                        let v = *ptrs[0].add(idx);
                        *state += v;
                    }
                    0
                })
                .unwrap();
            black_box(result)
        })
    });
}

fn bench_correlated_kernel(c: &mut Criterion) {
    let t1 = make_table(50_000, "a");
    let t2 = make_table(50_000, "b");
    let tables = vec![&t1, &t2];
    let schema = TableSchema::from_names(&["a", "b"]);
    let kernel = CorrelatedKernel::new(CorrelatedKernelConfig::full(t1.row_count()));

    c.bench_function("correlated_kernel", |b| {
        b.iter(|| {
            let result = kernel
                .run(
                    &tables,
                    schema.clone(),
                    0.0f64,
                    |idx, ptrs, _schema, state| {
                        unsafe {
                            let v = *ptrs[0].add(idx);
                            *state += v;
                        }
                        0
                    },
                )
                .unwrap();
            black_box(result)
        })
    });
}

fn bench_parallel_sweep(c: &mut Criterion) {
    let table = Arc::new(make_table(25_000, "close"));
    let params = vec![0.1, 0.2, 0.3, 0.4];

    c.bench_function("parallel_sweep", |b| {
        b.iter(|| {
            let result = par_run(table.clone(), params.clone(), |_p| {
                Box::new(|_idx, _ptrs, _state: &mut f64| 0)
            })
            .unwrap();
            black_box(result)
        })
    });
}

fn bench_simulation_engine_value(c: &mut Criterion) {
    use shape_runtime::simulation::{SimulationEngine, SimulationEngineConfig, StepResult};
    use shape_value::{ValueWord, ValueWordExt};

    let data: Vec<ValueWord> = (0..50_000).map(|i| ValueWord::from_f64(i as f64)).collect();
    let engine = SimulationEngine::new(SimulationEngineConfig::default());

    c.bench_function("simulation_engine_value", |b| {
        b.iter(|| {
            let result = engine
                .run(&data, |_value, state, _idx| {
                    Ok(StepResult::with_state(state.clone()))
                })
                .unwrap();
            black_box(result)
        })
    });
}

criterion_group!(
    benches,
    bench_dense_kernel_throughput,
    bench_correlated_kernel,
    bench_parallel_sweep,
    bench_simulation_engine_value
);
criterion_main!(benches);
