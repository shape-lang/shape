//! CorrelatedKernel - Multi-Table Simulation
//!
//! This module provides support for correlation analysis across multiple
//! aligned time series for cross-sensor analysis, multi-asset backtesting, etc.

use shape_ast::error::{Result, ShapeError};
use shape_value::DataTable;
use std::collections::HashMap;

/// Schema for multi-table context, defining table names and their order.
///
/// CRITICAL: Table order is fixed at schema creation time and used for
/// JIT compilation. The JIT compiler maps series names to indices at
/// compile time, enabling `context.temperature` → `series_ptrs[0][cursor_idx]`.
#[derive(Debug, Clone)]
pub struct TableSchema {
    /// Ordered list of table names (index = position in series_ptrs array)
    names: Vec<String>,
    /// Name to index mapping for compile-time resolution
    name_to_index: HashMap<String, usize>,
}

impl TableSchema {
    /// Create a new table schema from a list of names.
    ///
    /// The order of names determines their indices for JIT compilation.
    pub fn new(names: Vec<String>) -> Self {
        let name_to_index = names
            .iter()
            .enumerate()
            .map(|(idx, name)| (name.clone(), idx))
            .collect();
        Self {
            names,
            name_to_index,
        }
    }

    /// Create from a slice of string slices.
    pub fn from_names(names: &[&str]) -> Self {
        Self::new(names.iter().map(|s| s.to_string()).collect())
    }

    /// Get the index for a series name (used by JIT at compile time).
    #[inline]
    pub fn get_index(&self, name: &str) -> Option<usize> {
        self.name_to_index.get(name).copied()
    }

    /// Get the number of series.
    #[inline]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Check if schema is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Get all series names in order.
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Configuration for correlated kernel execution.
#[derive(Debug, Clone)]
pub struct CorrelatedKernelConfig {
    /// Start tick (inclusive)
    pub start: usize,
    /// End tick (exclusive)
    pub end: usize,
    /// Warmup period
    pub warmup: usize,
}

impl CorrelatedKernelConfig {
    /// Create a config for the full range.
    pub fn full(len: usize) -> Self {
        Self {
            start: 0,
            end: len,
            warmup: 0,
        }
    }

    /// Create with warmup period.
    pub fn with_warmup(len: usize, warmup: usize) -> Self {
        Self {
            start: 0,
            end: len,
            warmup,
        }
    }
}

/// Result of correlated kernel execution.
#[derive(Debug)]
pub struct CorrelatedKernelResult<S> {
    /// Final state after all ticks
    pub final_state: S,
    /// Number of ticks processed
    pub ticks_processed: usize,
    /// Whether simulation completed
    pub completed: bool,
}

/// The correlated simulation kernel for multi-series processing.
///
/// Enables correlation analysis across multiple aligned time series.
pub struct CorrelatedKernel {
    config: CorrelatedKernelConfig,
}

impl CorrelatedKernel {
    /// Create a new correlated kernel.
    pub fn new(config: CorrelatedKernelConfig) -> Self {
        Self { config }
    }

    /// Run correlated simulation across multiple DataTables.
    ///
    /// Each DataTable represents a separate series. All tables must have
    /// equal row counts. The strategy receives (tick_index, all_column_ptrs, state).
    #[inline(always)]
    pub fn run<S, F>(
        &self,
        tables: &[&DataTable],
        schema: TableSchema,
        mut initial_state: S,
        mut strategy: F,
    ) -> Result<CorrelatedKernelResult<S>>
    where
        F: FnMut(usize, &[*const f64], &TableSchema, &mut S) -> i32,
    {
        if tables.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "CorrelatedKernel requires at least one DataTable".to_string(),
                location: None,
            });
        }

        // Validate equal row counts
        let row_count = tables[0].row_count();
        for (i, table) in tables.iter().enumerate().skip(1) {
            if table.row_count() != row_count {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Table {} has {} rows but table 0 has {} rows",
                        i,
                        table.row_count(),
                        row_count
                    ),
                    location: None,
                });
            }
        }

        // Flatten all f64 column pointers across all tables
        let col_ptrs: Vec<*const f64> = tables
            .iter()
            .flat_map(|t| {
                t.column_ptrs()
                    .iter()
                    .filter(|cp| cp.stride == 8)
                    .map(|cp| cp.values_ptr as *const f64)
            })
            .collect();

        let effective_start = self.config.start + self.config.warmup;
        if effective_start >= self.config.end {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Warmup ({}) exceeds available range ({} - {})",
                    self.config.warmup, self.config.start, self.config.end
                ),
                location: None,
            });
        }

        let mut ticks_processed = 0;

        for cursor_index in effective_start..self.config.end {
            let result = strategy(cursor_index, &col_ptrs, &schema, &mut initial_state);
            if result != 0 {
                return Ok(CorrelatedKernelResult {
                    final_state: initial_state,
                    ticks_processed,
                    completed: result == 1,
                });
            }
            ticks_processed += 1;
        }

        Ok(CorrelatedKernelResult {
            final_state: initial_state,
            ticks_processed,
            completed: true,
        })
    }
}

/// Convenience function to run a correlated simulation.
pub fn simulate_correlated<S, F>(
    tables: &[&DataTable],
    schema: TableSchema,
    initial_state: S,
    strategy: F,
) -> Result<CorrelatedKernelResult<S>>
where
    F: FnMut(usize, &[*const f64], &TableSchema, &mut S) -> i32,
{
    if tables.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "simulate_correlated requires at least one DataTable".to_string(),
            location: None,
        });
    }
    let config = CorrelatedKernelConfig::full(tables[0].row_count());
    let kernel = CorrelatedKernel::new(config);
    kernel.run(tables, schema, initial_state, strategy)
}

/// JIT-compiled correlated kernel function type.
pub type CorrelatedKernelFn = unsafe extern "C" fn(
    cursor_index: usize,
    series_ptrs: *const *const f64,
    series_count: usize,
    state_ptr: *mut u8,
) -> i32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_series_schema_basic() {
        let schema = TableSchema::from_names(&["temp", "pressure"]);
        assert_eq!(schema.len(), 2);
        assert!(!schema.is_empty());
        assert_eq!(schema.get_index("temp"), Some(0));
        assert_eq!(schema.get_index("pressure"), Some(1));
        assert_eq!(schema.get_index("missing"), None);
        assert_eq!(
            schema.names(),
            &["temp".to_string(), "pressure".to_string()]
        );
    }

    #[test]
    fn test_series_schema_empty() {
        let schema = TableSchema::from_names(&[]);
        assert_eq!(schema.len(), 0);
        assert!(schema.is_empty());
        assert_eq!(schema.get_index("anything"), None);
    }

    /// Helper: build a DataTable with a single f64 column.
    fn make_f64_table(name: &str, values: Vec<f64>) -> DataTable {
        use arrow_array::{ArrayRef, Float64Array};
        use arrow_schema::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema = Schema::new(vec![Field::new(name, DataType::Float64, false)]);
        let col: ArrayRef = Arc::new(Float64Array::from(values));
        let batch = arrow_array::RecordBatch::try_new(Arc::new(schema), vec![col]).unwrap();
        DataTable::new(batch)
    }

    #[test]
    fn test_correlated_kernel_two_tables() {
        // Two tables: "spy" prices and "vix" values
        let spy_table = make_f64_table("price", vec![100.0, 102.0, 98.0, 105.0]);
        let vix_table = make_f64_table("value", vec![15.0, 25.0, 30.0, 12.0]);

        let schema = TableSchema::from_names(&["spy", "vix"]);
        let config = CorrelatedKernelConfig::full(spy_table.row_count());
        let kernel = CorrelatedKernel::new(config);

        // Strategy: when VIX > 20 and position == 0, buy; when VIX < 15, sell
        #[derive(Debug, Default)]
        struct State {
            position: f64,
            cash: f64,
            trades: u32,
        }

        let initial = State {
            position: 0.0,
            cash: 10000.0,
            trades: 0,
        };

        let tables: Vec<&DataTable> = vec![&spy_table, &vix_table];

        let result = kernel
            .run(&tables, schema, initial, |idx, col_ptrs, schema, state| {
                // col_ptrs[0] = spy price, col_ptrs[1] = vix value
                let spy_idx = schema.get_index("spy").unwrap();
                let vix_idx = schema.get_index("vix").unwrap();

                let spy_price = unsafe { *col_ptrs[spy_idx].add(idx) };
                let vix_value = unsafe { *col_ptrs[vix_idx].add(idx) };

                if vix_value > 20.0 && state.position == 0.0 {
                    // Buy
                    let shares = (state.cash / spy_price).floor();
                    state.cash -= shares * spy_price;
                    state.position = shares;
                    state.trades += 1;
                } else if vix_value < 15.0 && state.position > 0.0 {
                    // Sell
                    state.cash += state.position * spy_price;
                    state.position = 0.0;
                    state.trades += 1;
                }

                0 // continue
            })
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 4);
        // VIX=25 at idx 1: buy at SPY=102, shares = floor(10000/102) = 98
        // VIX=12 at idx 3: sell at SPY=105
        assert_eq!(result.final_state.trades, 2);
        assert_eq!(result.final_state.position, 0.0);
        // Bought 98 at 102 = 9996, remaining cash = 10000-9996 = 4
        // Sold 98 at 105 = 10290, total cash = 4 + 10290 = 10294
        assert_eq!(result.final_state.cash, 10294.0);
    }

    #[test]
    fn test_correlated_kernel_mismatched_rows() {
        let table1 = make_f64_table("a", vec![1.0, 2.0, 3.0]);
        let table2 = make_f64_table("b", vec![1.0, 2.0]); // different length

        let schema = TableSchema::from_names(&["a", "b"]);
        let tables: Vec<&DataTable> = vec![&table1, &table2];

        let result = simulate_correlated(&tables, schema, 0.0_f64, |_idx, _ptrs, _s, _st| 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_correlated_kernel_empty_tables() {
        let schema = TableSchema::from_names(&["a"]);
        let tables: Vec<&DataTable> = vec![];

        let result = simulate_correlated(&tables, schema, 0.0_f64, |_idx, _ptrs, _s, _st| 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_correlated_kernel_with_warmup() {
        let table1 = make_f64_table("a", vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let table2 = make_f64_table("b", vec![10.0, 20.0, 30.0, 40.0, 50.0]);

        let schema = TableSchema::from_names(&["a", "b"]);
        let config = CorrelatedKernelConfig::with_warmup(table1.row_count(), 2);
        let kernel = CorrelatedKernel::new(config);

        let tables: Vec<&DataTable> = vec![&table1, &table2];
        let mut visited = Vec::new();

        let result = kernel
            .run(&tables, schema, 0.0_f64, |idx, col_ptrs, _schema, state| {
                visited.push(idx);
                unsafe {
                    *state += *col_ptrs[0].add(idx) + *col_ptrs[1].add(idx);
                }
                0
            })
            .unwrap();

        assert!(result.completed);
        // Warmup=2, so should process indices 2, 3, 4
        assert_eq!(visited, vec![2, 3, 4]);
        // Sum: (3+30) + (4+40) + (5+50) = 33 + 44 + 55 = 132
        assert_eq!(result.final_state, 132.0);
    }

    #[test]
    fn test_correlated_kernel_early_stop() {
        let table1 = make_f64_table("a", vec![1.0, 2.0, 3.0, 4.0]);
        let table2 = make_f64_table("b", vec![10.0, 20.0, 30.0, 40.0]);

        let schema = TableSchema::from_names(&["a", "b"]);
        let config = CorrelatedKernelConfig::full(table1.row_count());
        let kernel = CorrelatedKernel::new(config);
        let tables: Vec<&DataTable> = vec![&table1, &table2];

        let result = kernel
            .run(&tables, schema, 0.0_f64, |idx, col_ptrs, _schema, state| {
                let val = unsafe { *col_ptrs[1].add(idx) };
                if val > 25.0 {
                    return 1; // done
                }
                *state += val;
                0
            })
            .unwrap();

        assert!(result.completed); // 1 = normal completion
        assert_eq!(result.ticks_processed, 2); // processed idx 0, 1; stopped at 2
        assert_eq!(result.final_state, 30.0); // 10+20
    }

    #[test]
    fn test_simulate_correlated_convenience() {
        let table1 = make_f64_table("a", vec![1.0, 2.0, 3.0]);
        let table2 = make_f64_table("b", vec![4.0, 5.0, 6.0]);

        let schema = TableSchema::from_names(&["a", "b"]);
        let tables: Vec<&DataTable> = vec![&table1, &table2];

        let result = simulate_correlated(&tables, schema, 0.0_f64, |idx, col_ptrs, _s, state| {
            unsafe {
                *state += *col_ptrs[0].add(idx) * *col_ptrs[1].add(idx);
            }
            0
        })
        .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 3);
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert_eq!(result.final_state, 32.0);
    }
}
