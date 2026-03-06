//! Task scheduler for the async host runtime.
//!
//! Manages spawned async tasks: stores their callables, tracks completion,
//! and executes them (synchronously for now) when the VM suspends on an await.
//!
//! The initial design runs tasks inline (synchronous execution at await-time).
//! True concurrent execution via Tokio can be layered on later by changing
//! `resolve_task` to spawn on the Tokio runtime.

use std::collections::HashMap;

use shape_value::{VMError, ValueWord};

/// Completion status of a spawned task.
#[derive(Debug, Clone)]
pub enum TaskStatus {
    /// Task has been spawned but not yet executed.
    Pending,
    /// Task finished successfully with a result value.
    Completed(ValueWord),
    /// Task was cancelled before completion.
    Cancelled,
}

/// Scheduler that tracks spawned async tasks by their future ID.
///
/// The VM's `SpawnTask` opcode registers a callable here. When the VM later
/// suspends on `WaitType::Future { id }`, the host looks up the callable,
/// executes it, and stores the result so the VM can resume.
#[derive(Debug)]
pub struct TaskScheduler {
    /// Map from task_id to the callable value (Closure or Function) that
    /// was passed to `spawn`. Consumed on first execution.
    callables: HashMap<u64, ValueWord>,

    /// Map from task_id to its completion status.
    results: HashMap<u64, TaskStatus>,
}

impl TaskScheduler {
    /// Create a new, empty scheduler.
    pub fn new() -> Self {
        Self {
            callables: HashMap::new(),
            results: HashMap::new(),
        }
    }

    /// Register a callable for a given task_id.
    ///
    /// Called by `op_spawn_task` when a new task is spawned.
    pub fn register(&mut self, task_id: u64, callable: ValueWord) {
        self.callables.insert(task_id, callable);
        self.results.insert(task_id, TaskStatus::Pending);
    }

    /// Take (remove) the callable for `task_id` so it can be executed.
    ///
    /// Returns `None` if the task was already consumed or never registered.
    pub fn take_callable(&mut self, task_id: u64) -> Option<ValueWord> {
        self.callables.remove(&task_id)
    }

    /// Record a completed result for a task.
    pub fn complete(&mut self, task_id: u64, value: ValueWord) {
        self.results.insert(task_id, TaskStatus::Completed(value));
    }

    /// Mark a task as cancelled.
    pub fn cancel(&mut self, task_id: u64) {
        // Only cancel if still pending
        if let Some(TaskStatus::Pending) = self.results.get(&task_id) {
            self.results.insert(task_id, TaskStatus::Cancelled);
            self.callables.remove(&task_id);
        }
    }

    /// Get the result for a task, if it has completed.
    pub fn get_result(&self, task_id: u64) -> Option<&TaskStatus> {
        self.results.get(&task_id)
    }

    /// Check whether a task has a stored result (completed or cancelled).
    pub fn is_resolved(&self, task_id: u64) -> bool {
        matches!(
            self.results.get(&task_id),
            Some(TaskStatus::Completed(_)) | Some(TaskStatus::Cancelled)
        )
    }

    /// Resolve a single task by executing its callable on a fresh VM executor.
    ///
    /// This is the synchronous (inline) strategy: the callable is executed
    /// immediately when awaited. Returns the result value, or an error.
    ///
    /// The `executor_fn` callback receives the callable ValueWord and must
    /// execute it, returning the result.
    pub fn resolve_task<F>(&mut self, task_id: u64, executor_fn: F) -> Result<ValueWord, VMError>
    where
        F: FnOnce(ValueWord) -> Result<ValueWord, VMError>,
    {
        // If already resolved, return the cached result
        if let Some(TaskStatus::Completed(val)) = self.results.get(&task_id) {
            return Ok(val.clone());
        }
        if let Some(TaskStatus::Cancelled) = self.results.get(&task_id) {
            return Err(VMError::RuntimeError(format!(
                "Task {} was cancelled",
                task_id
            )));
        }

        // Take the callable (consume it)
        let callable = self.take_callable(task_id).ok_or_else(|| {
            VMError::RuntimeError(format!("No callable registered for task {}", task_id))
        })?;

        // Execute synchronously
        let result = executor_fn(callable)?;

        // Cache the result
        self.results
            .insert(task_id, TaskStatus::Completed(result.clone()));

        Ok(result)
    }

    /// Resolve a task group according to the join strategy.
    ///
    /// Join kinds (encoded in the high 2 bits of JoinInit's packed operand):
    ///   0 = All  — wait for all tasks, return array of results
    ///   1 = Race — return first completed result
    ///   2 = Any  — return first successful result (skip errors)
    ///   3 = AllSettled — return array of {status, value/error} for every task
    ///
    /// Since we execute synchronously, "race" and "any" still run all tasks
    /// sequentially but return early on the first applicable result.
    pub fn resolve_task_group<F>(
        &mut self,
        kind: u8,
        task_ids: &[u64],
        mut executor_fn: F,
    ) -> Result<ValueWord, VMError>
    where
        F: FnMut(ValueWord) -> Result<ValueWord, VMError>,
    {
        match kind {
            // All: collect all results into an array
            0 => {
                let mut results: Vec<ValueWord> = Vec::with_capacity(task_ids.len());
                for &id in task_ids {
                    let val = self.resolve_task(id, &mut executor_fn)?;
                    results.push(val);
                }
                Ok(ValueWord::from_array(std::sync::Arc::new(results)))
            }
            // Race: return first result (all run, but we return first)
            1 => {
                for &id in task_ids {
                    let val = self.resolve_task(id, &mut executor_fn)?;
                    return Ok(val);
                }
                Err(VMError::RuntimeError(
                    "Race join with empty task list".to_string(),
                ))
            }
            // Any: return first success, skip errors
            2 => {
                let mut last_err = None;
                for &id in task_ids {
                    match self.resolve_task(id, &mut executor_fn) {
                        Ok(val) => return Ok(val),
                        Err(e) => last_err = Some(e),
                    }
                }
                Err(last_err.unwrap_or_else(|| {
                    VMError::RuntimeError("Any join with empty task list".to_string())
                }))
            }
            // AllSettled: collect {status, value/error} for each
            3 => {
                let mut results: Vec<ValueWord> = Vec::with_capacity(task_ids.len());
                for &id in task_ids {
                    match self.resolve_task(id, &mut executor_fn) {
                        Ok(val) => results.push(val),
                        Err(e) => results.push(ValueWord::from_string(std::sync::Arc::new(
                            format!("Error: {}", e),
                        ))),
                    }
                }
                Ok(ValueWord::from_array(std::sync::Arc::new(results)))
            }
            _ => Err(VMError::RuntimeError(format!(
                "Unknown join kind: {}",
                kind
            ))),
        }
    }
}

#[cfg(feature = "gc")]
impl TaskScheduler {
    /// Scan all heap-referencing ValueWord roots held by the scheduler.
    ///
    /// Called during GC root enumeration. Both pending callables and
    /// completed results may reference heap objects.
    pub(crate) fn scan_roots(&self, visitor: &mut dyn FnMut(*mut u8)) {
        for callable in self.callables.values() {
            shape_gc::roots::trace_nanboxed_bits(callable.raw_bits(), visitor);
        }
        for status in self.results.values() {
            if let TaskStatus::Completed(val) = status {
                shape_gc::roots::trace_nanboxed_bits(val.raw_bits(), visitor);
            }
        }
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_register_and_take_callable() {
        let mut sched = TaskScheduler::new();
        sched.register(1, ValueWord::from_function(42));
        assert!(matches!(sched.get_result(1), Some(TaskStatus::Pending)));

        let callable = sched.take_callable(1);
        assert!(callable.is_some());

        // Second take returns None (consumed)
        assert!(sched.take_callable(1).is_none());
    }

    #[test]
    fn test_resolve_task_synchronous() {
        let mut sched = TaskScheduler::new();
        sched.register(1, ValueWord::from_function(0));

        let result = sched.resolve_task(1, |_callable| Ok(ValueWord::from_f64(99.0)));
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!((val.as_f64().unwrap() - 99.0).abs() < f64::EPSILON);

        // Second resolve returns cached result
        let cached = sched.resolve_task(1, |_| panic!("should not be called"));
        assert!(cached.is_ok());
    }

    #[test]
    fn test_cancel_task() {
        let mut sched = TaskScheduler::new();
        sched.register(1, ValueWord::from_function(0));

        sched.cancel(1);
        assert!(sched.is_resolved(1));

        let result = sched.resolve_task(1, |_| Ok(ValueWord::none()));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_all_group() {
        let mut sched = TaskScheduler::new();
        sched.register(1, ValueWord::from_function(0));
        sched.register(2, ValueWord::from_function(1));

        let mut call_count = 0u32;
        let result = sched.resolve_task_group(0, &[1, 2], |_callable| {
            call_count += 1;
            Ok(ValueWord::from_f64(call_count as f64))
        });
        assert!(result.is_ok());
        let val = result.unwrap();
        let view = val.as_any_array().expect("Expected array");
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn test_resolve_race_group() {
        let mut sched = TaskScheduler::new();
        sched.register(10, ValueWord::from_function(0));
        sched.register(20, ValueWord::from_function(1));

        let result = sched.resolve_task_group(1, &[10, 20], |_| {
            Ok(ValueWord::from_string(Arc::new("first".to_string())))
        });
        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val.as_str().unwrap(), "first");
    }
}
