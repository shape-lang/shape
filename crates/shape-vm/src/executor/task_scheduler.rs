//! Task scheduler for the async host runtime.
//!
//! Manages spawned async tasks: stores their callables, tracks completion,
//! and executes them (synchronously for now) when the VM suspends on an await.
//!
//! The initial design runs tasks inline (synchronous execution at await-time).
//! True concurrent execution via Tokio can be layered on later by changing
//! `resolve_task` to spawn on the Tokio runtime.

use std::collections::HashMap;

use shape_value::{VMError, ValueWord, ValueWordExt};

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
///
/// Supports both inline tasks (callable executed synchronously at await-time)
/// and external tasks (completed by background Tokio tasks via oneshot channels).
pub struct TaskScheduler {
    /// Map from task_id to the callable value (Closure or Function) that
    /// was passed to `spawn`. Consumed on first execution.
    callables: HashMap<u64, ValueWord>,

    /// Map from task_id to its completion status.
    results: HashMap<u64, TaskStatus>,

    /// External completion channels — Tokio background tasks send results here.
    /// Used for remote calls and other externally-completed futures.
    external_receivers: HashMap<u64, tokio::sync::oneshot::Receiver<Result<ValueWord, String>>>,
}

impl TaskScheduler {
    /// Create a new, empty scheduler.
    pub fn new() -> Self {
        Self {
            callables: HashMap::new(),
            results: HashMap::new(),
            external_receivers: HashMap::new(),
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

    /// Register an externally-completed task (e.g., remote call).
    ///
    /// Returns a `oneshot::Sender` that the background task uses to deliver the
    /// result. The scheduler marks the task as Pending and stores the receiver.
    pub fn register_external(
        &mut self,
        task_id: u64,
    ) -> tokio::sync::oneshot::Sender<Result<ValueWord, String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.results.insert(task_id, TaskStatus::Pending);
        self.external_receivers.insert(task_id, rx);
        tx
    }

    /// Try to resolve an external task (non-blocking check).
    ///
    /// Returns `Some(Ok(val))` if the external task completed successfully,
    /// `Some(Err(..))` on error/cancellation, or `None` if still pending.
    pub fn try_resolve_external(&mut self, task_id: u64) -> Option<Result<ValueWord, VMError>> {
        if let Some(TaskStatus::Completed(val)) = self.results.get(&task_id) {
            return Some(Ok(val.clone()));
        }
        if let Some(rx) = self.external_receivers.get_mut(&task_id) {
            match rx.try_recv() {
                Ok(Ok(val)) => {
                    self.results
                        .insert(task_id, TaskStatus::Completed(val.clone()));
                    self.external_receivers.remove(&task_id);
                    Some(Ok(val))
                }
                Ok(Err(e)) => {
                    self.external_receivers.remove(&task_id);
                    Some(Err(VMError::RuntimeError(e)))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.external_receivers.remove(&task_id);
                    Some(Err(VMError::RuntimeError(
                        "Remote task cancelled".to_string(),
                    )))
                }
            }
        } else {
            None
        }
    }

    /// Check whether a task has an external receiver (is externally-completed).
    pub fn has_external(&self, task_id: u64) -> bool {
        self.external_receivers.contains_key(&task_id)
    }

    /// Take the external receiver for async awaiting.
    ///
    /// Used by `execute_with_async` when it needs to truly `.await` an external
    /// task's completion.
    pub fn take_external_receiver(
        &mut self,
        task_id: u64,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<ValueWord, String>>> {
        self.external_receivers.remove(&task_id)
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
                Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)))
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
                Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)))
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

impl std::fmt::Debug for TaskScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskScheduler")
            .field("callables", &self.callables)
            .field("results", &self.results)
            .field(
                "external_receivers",
                &format!("[{} pending]", self.external_receivers.len()),
            )
            .finish()
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

    #[test]
    fn test_register_external_and_resolve() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(100);
        assert!(sched.has_external(100));
        assert!(matches!(sched.get_result(100), Some(TaskStatus::Pending)));

        // Not yet resolved
        assert!(sched.try_resolve_external(100).is_none());

        // Send result from "background task"
        tx.send(Ok(ValueWord::from_f64(42.0))).unwrap();

        // Now resolves
        let result = sched.try_resolve_external(100);
        assert!(result.is_some());
        let val = result.unwrap().unwrap();
        assert!((val.as_f64().unwrap() - 42.0).abs() < f64::EPSILON);

        // Receiver removed after resolution
        assert!(!sched.has_external(100));
    }

    #[test]
    fn test_external_task_error() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(200);

        tx.send(Err("connection refused".to_string())).unwrap();

        let result = sched.try_resolve_external(200);
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_external_task_cancelled() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(300);

        // Drop sender to simulate cancellation
        drop(tx);

        let result = sched.try_resolve_external(300);
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_take_external_receiver() {
        let mut sched = TaskScheduler::new();
        let _tx = sched.register_external(400);

        assert!(sched.has_external(400));
        let rx = sched.take_external_receiver(400);
        assert!(rx.is_some());
        assert!(!sched.has_external(400));
    }
}
