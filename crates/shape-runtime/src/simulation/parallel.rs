//! Parallel Parameter Sweeps
//!
//! This module provides parallel execution utilities for running multiple
//! simulations with different parameter sets using rayon for parallelization.

use super::dense_kernel::{DenseKernel, DenseKernelConfig, DenseKernelResult};
use shape_ast::error::Result;
use shape_value::DataTable;
use std::sync::Arc;

/// Result of a parallel parameter sweep.
#[derive(Debug)]
pub struct ParallelSweepResult<S, P> {
    /// Results for each parameter set
    pub results: Vec<(P, DenseKernelResult<S>)>,
    /// Number of simulations run
    pub simulations_run: usize,
    /// Total ticks processed across all simulations
    pub total_ticks: usize,
}

/// Run a parameter sweep in parallel using rayon.
///
/// This function runs multiple simulations in parallel, each with different
/// parameter values. Data is shared (zero-copy) across all simulations using Arc.
pub fn par_run<P, S, F>(
    data: Arc<DataTable>,
    param_sets: Vec<P>,
    strategy_factory: F,
) -> Result<ParallelSweepResult<S, P>>
where
    P: Send + Sync + Clone,
    S: Send + Default,
    F: Fn(&P) -> Box<dyn FnMut(usize, &[*const f64], &mut S) -> i32 + Send> + Send + Sync,
{
    let config = DenseKernelConfig::full(data.row_count());
    par_run_with_config(data, param_sets, config, strategy_factory)
}

/// Run a parameter sweep with custom DenseKernelConfig.
pub fn par_run_with_config<P, S, F>(
    data: Arc<DataTable>,
    param_sets: Vec<P>,
    config: DenseKernelConfig,
    strategy_factory: F,
) -> Result<ParallelSweepResult<S, P>>
where
    P: Send + Sync + Clone,
    S: Send + Default,
    F: Fn(&P) -> Box<dyn FnMut(usize, &[*const f64], &mut S) -> i32 + Send> + Send + Sync,
{
    use rayon::prelude::*;

    let results: Vec<(P, DenseKernelResult<S>)> = param_sets
        .par_iter()
        .map(|params| {
            let kernel = DenseKernel::new(config.clone());
            let mut strategy = strategy_factory(params);
            let state = S::default();
            let result = kernel.run(&data, state, |idx, ptrs, s| strategy(idx, ptrs, s));
            // If simulation errored, create a default result
            let result = result.unwrap_or(DenseKernelResult {
                final_state: S::default(),
                ticks_processed: 0,
                completed: false,
            });
            (params.clone(), result)
        })
        .collect();

    let simulations_run = results.len();
    let total_ticks = results.iter().map(|(_, r)| r.ticks_processed).sum();

    Ok(ParallelSweepResult {
        results,
        simulations_run,
        total_ticks,
    })
}

/// Build a 2D parameter grid.
pub fn param_grid<A, B>(a_values: Vec<A>, b_values: Vec<B>) -> Vec<(A, B)>
where
    A: Clone,
    B: Clone,
{
    let mut grid = Vec::with_capacity(a_values.len() * b_values.len());
    for a in &a_values {
        for b in &b_values {
            grid.push((a.clone(), b.clone()));
        }
    }
    grid
}

/// Build a 3D parameter grid.
pub fn param_grid3<A, B, C>(a_values: Vec<A>, b_values: Vec<B>, c_values: Vec<C>) -> Vec<(A, B, C)>
where
    A: Clone,
    B: Clone,
    C: Clone,
{
    let mut grid = Vec::with_capacity(a_values.len() * b_values.len() * c_values.len());
    for a in &a_values {
        for b in &b_values {
            for c in &c_values {
                grid.push((a.clone(), b.clone(), c.clone()));
            }
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_grid() {
        let grid = param_grid(vec![1, 2], vec![10, 20, 30]);
        assert_eq!(grid.len(), 6);
        assert_eq!(grid[0], (1, 10));
        assert_eq!(grid[5], (2, 30));
    }

    #[test]
    fn test_param_grid3() {
        let grid = param_grid3(vec![1, 2], vec![10, 20], vec![100, 200]);
        assert_eq!(grid.len(), 8); // 2 × 2 × 2
        assert_eq!(grid[0], (1, 10, 100));
        assert_eq!(grid[7], (2, 20, 200));
    }
}
