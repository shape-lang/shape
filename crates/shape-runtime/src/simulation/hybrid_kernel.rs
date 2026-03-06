//! HybridKernel - Event-Driven Simulation
//!
//! This module combines tick-by-tick processing with discrete event scheduling,
//! enabling complex simulations with both regular updates and scheduled events.

use super::event_scheduler::{EventQueue, ScheduledEvent};
use shape_ast::error::{Result, ShapeError};
use shape_value::DataTable;

/// Result of hybrid kernel execution.
#[derive(Debug)]
pub struct HybridKernelResult<S> {
    /// Final state after all processing
    pub final_state: S,
    /// Number of ticks processed
    pub ticks_processed: usize,
    /// Number of events processed
    pub events_processed: usize,
    /// Whether simulation completed
    pub completed: bool,
}

/// Configuration for hybrid kernel execution.
#[derive(Debug, Clone)]
pub struct HybridKernelConfig {
    /// Start tick (inclusive)
    pub start: usize,
    /// End tick (exclusive)
    pub end: usize,
    /// Warmup period
    pub warmup: usize,
}

impl HybridKernelConfig {
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

/// Event handler function type.
pub type EventHandlerFn<S> = fn(&ScheduledEvent, &mut S, &mut EventQueue) -> Result<()>;

/// Hybrid simulation kernel combining tick-by-tick and event-driven processing.
pub struct HybridKernel {
    config: HybridKernelConfig,
}

impl HybridKernel {
    /// Create a new hybrid kernel.
    pub fn new(config: HybridKernelConfig) -> Self {
        Self { config }
    }

    /// Run hybrid simulation on a DataTable with tick strategy and event handler.
    ///
    /// Each tick: call `tick_strategy`, then process any due events via `event_handler`.
    pub fn run<S, F>(
        &self,
        data: &DataTable,
        mut initial_state: S,
        mut event_queue: EventQueue,
        mut tick_strategy: F,
        event_handler: EventHandlerFn<S>,
    ) -> Result<HybridKernelResult<S>>
    where
        F: FnMut(usize, &[*const f64], &mut S) -> i32,
    {
        let col_ptrs: Vec<*const f64> = data
            .column_ptrs()
            .iter()
            .filter(|cp| cp.stride == 8)
            .map(|cp| cp.values_ptr as *const f64)
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
        let mut events_processed = 0;

        for cursor_index in effective_start..self.config.end {
            // Process tick
            let result = tick_strategy(cursor_index, &col_ptrs, &mut initial_state);
            if result != 0 {
                return Ok(HybridKernelResult {
                    final_state: initial_state,
                    ticks_processed,
                    events_processed,
                    completed: result == 1,
                });
            }
            ticks_processed += 1;

            // Process any due events at this tick
            while let Some(event) = event_queue.pop_due(cursor_index as i64) {
                event_handler(&event, &mut initial_state, &mut event_queue)?;
                events_processed += 1;
            }
        }

        Ok(HybridKernelResult {
            final_state: initial_state,
            ticks_processed,
            events_processed,
            completed: true,
        })
    }
}

/// Convenience function to run a hybrid simulation.
pub fn simulate_hybrid<S, F>(
    data: &DataTable,
    initial_state: S,
    event_queue: EventQueue,
    tick_strategy: F,
    event_handler: EventHandlerFn<S>,
) -> Result<HybridKernelResult<S>>
where
    F: FnMut(usize, &[*const f64], &mut S) -> i32,
{
    let config = HybridKernelConfig::full(data.row_count());
    let kernel = HybridKernel::new(config);
    kernel.run(
        data,
        initial_state,
        event_queue,
        tick_strategy,
        event_handler,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_hybrid_kernel_ticks_only() {
        // No events scheduled - pure tick processing
        let table = make_f64_table("price", vec![10.0, 20.0, 30.0]);
        let event_queue = EventQueue::new();

        fn no_op_handler(
            _event: &ScheduledEvent,
            _state: &mut f64,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            panic!("Should not be called with no events");
        }

        let result = simulate_hybrid(
            &table,
            0.0_f64,
            event_queue,
            |idx, col_ptrs, state| {
                unsafe { *state += *col_ptrs[0].add(idx) };
                0
            },
            no_op_handler,
        )
        .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 3);
        assert_eq!(result.events_processed, 0);
        assert_eq!(result.final_state, 60.0); // 10+20+30
    }

    #[test]
    fn test_hybrid_kernel_with_scheduled_events() {
        let table = make_f64_table("price", vec![100.0, 105.0, 110.0, 108.0, 112.0]);

        let mut event_queue = EventQueue::new();
        // Schedule event at tick 2 (rebalance)
        event_queue.schedule(2, 1, 0);
        // Schedule event at tick 4 (final rebalance)
        event_queue.schedule(4, 1, 0);

        #[derive(Debug, Default)]
        struct State {
            sum: f64,
            rebalance_count: u32,
        }

        fn rebalance_handler(
            _event: &ScheduledEvent,
            state: &mut State,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            state.rebalance_count += 1;
            Ok(())
        }

        let config = HybridKernelConfig::full(table.row_count());
        let kernel = HybridKernel::new(config);

        let result = kernel
            .run(
                &table,
                State::default(),
                event_queue,
                |idx, col_ptrs, state| {
                    unsafe { state.sum += *col_ptrs[0].add(idx) };
                    0
                },
                rebalance_handler,
            )
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 5);
        assert_eq!(result.events_processed, 2);
        assert_eq!(result.final_state.rebalance_count, 2);
        assert_eq!(result.final_state.sum, 535.0); // 100+105+110+108+112
    }

    #[test]
    fn test_hybrid_kernel_event_spawns_event() {
        // Test that an event handler can schedule new events
        let table = make_f64_table("price", vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        let mut event_queue = EventQueue::new();
        // Schedule an event at tick 1
        event_queue.schedule(1, 1, 0);

        #[derive(Debug, Default)]
        struct State {
            events_seen: u32,
        }

        fn cascading_handler(
            event: &ScheduledEvent,
            state: &mut State,
            queue: &mut EventQueue,
        ) -> Result<()> {
            state.events_seen += 1;
            // If this is the first event (at tick 1), schedule another at tick 3
            if event.time == 1 {
                queue.schedule(3, 2, 0);
            }
            Ok(())
        }

        let result = simulate_hybrid(
            &table,
            State::default(),
            event_queue,
            |_idx, _col_ptrs, _state| 0,
            cascading_handler,
        )
        .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 5);
        assert_eq!(result.events_processed, 2); // original + spawned
        assert_eq!(result.final_state.events_seen, 2);
    }

    #[test]
    fn test_hybrid_kernel_multiple_events_same_tick() {
        let table = make_f64_table("price", vec![1.0, 2.0, 3.0]);

        let mut event_queue = EventQueue::new();
        // Schedule 3 events at the same tick
        event_queue.schedule(1, 10, 0);
        event_queue.schedule(1, 20, 0);
        event_queue.schedule(1, 30, 0);

        fn counting_handler(
            _event: &ScheduledEvent,
            state: &mut u32,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            *state += 1;
            Ok(())
        }

        let result = simulate_hybrid(
            &table,
            0_u32,
            event_queue,
            |_idx, _col_ptrs, _state| 0,
            counting_handler,
        )
        .unwrap();

        assert!(result.completed);
        assert_eq!(result.events_processed, 3);
        assert_eq!(result.final_state, 3);
    }

    #[test]
    fn test_hybrid_kernel_tick_early_stop() {
        let table = make_f64_table("price", vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        let mut event_queue = EventQueue::new();
        event_queue.schedule(4, 1, 0); // event at tick 4 (should not fire)

        fn noop_handler(
            _event: &ScheduledEvent,
            _state: &mut u32,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            panic!("Should not fire - tick stops before tick 4");
        }

        let result = simulate_hybrid(
            &table,
            0_u32,
            event_queue,
            |idx, _col_ptrs, state| {
                *state += 1;
                if idx == 2 {
                    1 // stop
                } else {
                    0
                }
            },
            noop_handler,
        )
        .unwrap();

        assert!(result.completed); // result == 1 means normal completion
        assert_eq!(result.ticks_processed, 2); // 0, 1 (tick 2 returned non-zero so not counted)
        assert_eq!(result.events_processed, 0);
    }

    #[test]
    fn test_hybrid_kernel_with_warmup() {
        let table = make_f64_table("price", vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        let mut event_queue = EventQueue::new();
        // Event at tick 0 (within warmup, should not fire)
        event_queue.schedule(0, 1, 0);
        // Event at tick 3 (after warmup, should fire)
        event_queue.schedule(3, 2, 0);

        fn handler(
            _event: &ScheduledEvent,
            state: &mut u32,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            *state += 100;
            Ok(())
        }

        let config = HybridKernelConfig::with_warmup(table.row_count(), 2);
        let kernel = HybridKernel::new(config);

        let result = kernel
            .run(
                &table,
                0_u32,
                event_queue,
                |_idx, _col_ptrs, state| {
                    *state += 1;
                    0
                },
                handler,
            )
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.ticks_processed, 3); // indices 2, 3, 4
        // Event at tick 0 is "due" when we first check at tick 2 (0 <= 2), so it fires
        // Event at tick 3 fires after tick 3
        assert_eq!(result.events_processed, 2);
        // 3 ticks + 2 events*100 = 3 + 200 = 203
        assert_eq!(result.final_state, 203);
    }

    #[test]
    fn test_hybrid_kernel_warmup_exceeds_range() {
        let table = make_f64_table("price", vec![1.0, 2.0]);
        let config = HybridKernelConfig::with_warmup(table.row_count(), 10);
        let kernel = HybridKernel::new(config);

        fn noop_handler(
            _event: &ScheduledEvent,
            _state: &mut f64,
            _queue: &mut EventQueue,
        ) -> Result<()> {
            Ok(())
        }

        let result = kernel.run(
            &table,
            0.0_f64,
            EventQueue::new(),
            |_idx, _col_ptrs, _state| 0,
            noop_handler,
        );
        assert!(result.is_err());
    }
}
