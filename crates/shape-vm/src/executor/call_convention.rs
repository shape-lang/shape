//! Function and closure call convention, execution wrappers, and async resolution.

use shape_value::{Upvalue, VMError, ValueWord};

use super::{CallFrame, ExecutionResult, VirtualMachine, task_scheduler};

impl VirtualMachine {
    /// Execute a named function with arguments, returning its result.
    ///
    /// If the program has module-level bindings, the top-level code is executed
    /// first (once) to initialize them before calling the target function.
    pub fn execute_function_by_name(
        &mut self,
        name: &str,
        args: Vec<ValueWord>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        let func_id = self
            .program
            .functions
            .iter()
            .position(|f| f.name == name)
            .ok_or_else(|| VMError::RuntimeError(format!("Function '{}' not found", name)))?;

        // Run the top-level code first to initialize module bindings,
        // but only if there are module bindings that need initialization.
        if !self.program.module_binding_names.is_empty() && !self.module_init_done {
            self.reset();
            self.execute(None)?;
            self.module_init_done = true;
        }

        // Now call the target function.
        // Use reset_stack to keep module_bindings intact.
        self.reset_stack();
        self.ip = self.program.instructions.len();
        self.call_function_with_nb_args(func_id as u16, &args)?;
        self.execute(ctx)
    }

    /// Execute a function by its ID with positional arguments.
    ///
    /// Used by the remote execution system when the caller already knows the
    /// function index (e.g., from a `RemoteCallRequest.function_id`).
    pub fn execute_function_by_id(
        &mut self,
        func_id: u16,
        args: Vec<ValueWord>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        self.reset();
        self.ip = self.program.instructions.len();
        self.call_function_with_nb_args(func_id, &args)?;
        self.execute(ctx)
    }

    /// Execute a closure with its captured upvalues and arguments.
    ///
    /// Used by the remote execution system to run closures that were
    /// serialized with their captured values.
    pub fn execute_closure(
        &mut self,
        function_id: u16,
        upvalues: Vec<Upvalue>,
        args: Vec<ValueWord>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        self.reset();
        self.ip = self.program.instructions.len();
        self.call_closure_with_nb_args(function_id, upvalues, &args)?;
        self.execute(ctx)
    }

    /// Fast function execution for hot loops (backtesting)
    /// - Uses pre-computed function ID (no name lookup)
    /// - Uses reset_minimal() for minimum overhead
    /// - Uses execute_fast() which skips debugging overhead
    /// - Assumes function doesn't create GC objects or use exceptions
    pub fn execute_function_fast(
        &mut self,
        func_id: u16,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        // Minimal reset - only essential state, no GC overhead
        self.reset_minimal();
        self.ip = self.program.instructions.len();
        self.call_function_with_nb_args(func_id, &[])?;
        self.execute_fast(ctx)
    }

    /// Execute a function with named arguments
    /// Maps named args to positional based on function's param_names
    pub fn execute_function_with_named_args(
        &mut self,
        func_id: u16,
        named_args: &[(String, ValueWord)],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        let function = self
            .program
            .functions
            .get(func_id as usize)
            .ok_or(VMError::InvalidCall)?;

        // Map named args to positional based on param_names
        let mut args = vec![ValueWord::none(); function.arity as usize];
        for (name, value) in named_args {
            if let Some(idx) = function.param_names.iter().position(|p| p == name) {
                if idx < args.len() {
                    args[idx] = value.clone();
                }
            }
        }

        self.reset_minimal();
        self.ip = self.program.instructions.len();
        self.call_function_with_nb_args(func_id, &args)?;
        self.execute_fast(ctx)
    }

    /// Resume execution after a suspension.
    ///
    /// The resolved value is pushed onto the stack, and execution continues
    /// from where it left off (the IP is already set to the resume point).
    pub fn resume(
        &mut self,
        value: ValueWord,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ExecutionResult, VMError> {
        self.push_vw(value)?;
        self.execute_with_suspend(ctx)
    }

    /// Execute with automatic async task resolution.
    ///
    /// Runs `execute_with_suspend` in a loop. Each time the VM suspends on a
    /// `Future { id }`, the host resolves the task via the TaskScheduler
    /// (synchronously executing the spawned callable inline) and resumes the
    /// VM with the result. This continues until execution completes or an
    /// unresolvable suspension is encountered.
    pub fn execute_with_async(
        &mut self,
        mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        loop {
            match self.execute_with_suspend(ctx.as_deref_mut())? {
                ExecutionResult::Completed(value) => return Ok(value),
                ExecutionResult::Suspended { future_id, .. } => {
                    // Try to resolve via the task scheduler
                    let result = self.resolve_spawned_task(future_id)?;
                    // Push the result so the resumed VM finds it on the stack
                    self.push_vw(result)?;
                    // Loop continues with execute_with_suspend
                }
            }
        }
    }

    /// Resolve a spawned task by executing its callable synchronously.
    ///
    /// Looks up the callable in the TaskScheduler, then executes it:
    /// - NanTag::Function -> calls via call_function_with_nb_args
    /// - HeapValue::Closure -> calls via call_closure_with_nb_args
    /// - Other values -> returns them directly (already-resolved value)
    ///
    /// For externally-completed tasks (remote calls), checks the oneshot
    /// receiver first (non-blocking).
    fn resolve_spawned_task(&mut self, task_id: u64) -> Result<ValueWord, VMError> {
        // Check if already resolved (cached)
        if let Some(task_scheduler::TaskStatus::Completed(val)) =
            self.task_scheduler.get_result(task_id)
        {
            return Ok(val.clone());
        }
        if let Some(task_scheduler::TaskStatus::Cancelled) = self.task_scheduler.get_result(task_id)
        {
            return Err(VMError::RuntimeError(format!(
                "Task {} was cancelled",
                task_id
            )));
        }

        // Check external receivers (non-blocking) before inline execution
        if let Some(result) = self.task_scheduler.try_resolve_external(task_id) {
            return result;
        }

        // If this is an external task that hasn't completed yet, block on it
        // using tokio's block_in_place to avoid deadlocking the runtime.
        if self.task_scheduler.has_external(task_id) {
            if let Some(rx) = self.task_scheduler.take_external_receiver(task_id) {
                let result =
                    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(rx))
                        .map_err(|_| VMError::RuntimeError("Remote task dropped".to_string()))?
                        .map_err(VMError::RuntimeError)?;
                self.task_scheduler.complete(task_id, result.clone());
                return Ok(result);
            }
        }

        // Take the callable
        let callable_nb = self.task_scheduler.take_callable(task_id).ok_or_else(|| {
            VMError::RuntimeError(format!("No callable registered for task {}", task_id))
        })?;

        // Execute based on callable type.
        // We save/restore the instruction pointer and stack depth so the
        // nested execution doesn't corrupt the outer (suspended) state.
        use shape_value::NanTag;

        let result_nb = match callable_nb.tag() {
            NanTag::Function => {
                let func_id = callable_nb.as_function().ok_or(VMError::InvalidCall)?;
                let saved_ip = self.ip;
                let saved_sp = self.sp;

                self.ip = self.program.instructions.len();
                self.call_function_with_nb_args(func_id, &[])?;
                let res = self.execute_fast(None);

                self.ip = saved_ip;
                // Restore stack pointer (clear anything left above saved_sp)
                for i in saved_sp..self.sp {
                    self.stack[i] = ValueWord::none();
                }
                self.sp = saved_sp;

                res?
            }
            NanTag::Heap => {
                if let Some((function_id, upvalues)) = callable_nb.as_closure() {
                    let upvalues = upvalues.to_vec();
                    let saved_ip = self.ip;
                    let saved_sp = self.sp;

                    self.ip = self.program.instructions.len();
                    self.call_closure_with_nb_args(function_id, upvalues, &[])?;
                    let res = self.execute_fast(None);

                    self.ip = saved_ip;
                    for i in saved_sp..self.sp {
                        self.stack[i] = ValueWord::none();
                    }
                    self.sp = saved_sp;

                    res?
                } else {
                    // If someone spawned an already-resolved value, just return it
                    callable_nb
                }
            }
            // If someone spawned an already-resolved value, just return it
            _ => callable_nb,
        };

        // Cache the result
        self.task_scheduler.complete(task_id, result_nb.clone());

        Ok(result_nb)
    }

    /// ValueWord-module function call: takes ValueWord args directly.
    pub(crate) fn call_function_with_nb_args(
        &mut self,
        func_id: u16,
        args: &[ValueWord],
    ) -> Result<(), VMError> {
        let function = self
            .program
            .functions
            .get(func_id as usize)
            .ok_or(VMError::InvalidCall)?;

        if self.call_stack.len() >= self.config.max_call_depth {
            return Err(VMError::StackOverflow);
        }

        let locals_count = function.locals_count as usize;
        let param_count = function.arity as usize;
        let entry_point = function.entry_point;
        let ref_params = function.ref_params.clone();

        // Count ref params that need shadow slots for their actual values.
        // DerefLoad/DerefStore expect the param slot to contain a TAG_REF
        // pointing to a *different* slot that holds the real value.
        let ref_shadow_count = ref_params
            .iter()
            .enumerate()
            .filter(|&(i, &is_ref)| is_ref && i < param_count && i < locals_count)
            .count();

        let bp = self.sp;
        let total_slots = locals_count + ref_shadow_count;
        let needed = bp + total_slots;
        if needed > self.stack.len() {
            self.stack.resize_with(needed * 2 + 1, ValueWord::none);
        }

        for i in 0..param_count {
            if i < locals_count {
                self.stack[bp + i] = args.get(i).cloned().unwrap_or_else(ValueWord::none);
            }
        }

        // For ref-inferred parameters: move the actual value to a shadow slot
        // beyond locals_count, then replace the param slot with a TAG_REF
        // pointing to the shadow slot. This way DerefLoad follows the ref
        // to the actual value (not a circular self-reference).
        let mut shadow_idx = 0;
        for (i, &is_ref) in ref_params.iter().enumerate() {
            if is_ref && i < param_count && i < locals_count {
                let shadow_slot = bp + locals_count + shadow_idx;
                self.stack[shadow_slot] = self.stack[bp + i].clone();
                self.stack[bp + i] = ValueWord::from_ref(shadow_slot);
                shadow_idx += 1;
            }
        }

        self.sp = needed;

        let blob_hash = self.blob_hash_for_function(func_id);
        let frame = CallFrame {
            return_ip: self.ip,
            base_pointer: bp,
            locals_count: total_slots,
            function_id: Some(func_id),
            upvalues: None,
            blob_hash,
        };
        self.call_stack.push(frame);
        self.ip = entry_point;
        Ok(())
    }

    /// ValueWord-host closure call: takes ValueWord args directly.
    pub(crate) fn call_closure_with_nb_args(
        &mut self,
        func_id: u16,
        upvalues: Vec<Upvalue>,
        args: &[ValueWord],
    ) -> Result<(), VMError> {
        let function = self
            .program
            .functions
            .get(func_id as usize)
            .ok_or(VMError::InvalidCall)?;

        if self.call_stack.len() >= self.config.max_call_depth {
            return Err(VMError::StackOverflow);
        }

        let locals_count = function.locals_count as usize;
        let captures_count = function.captures_count as usize;
        let arity = function.arity as usize;
        let entry_point = function.entry_point;

        let bp = self.sp;
        let needed = bp + locals_count;
        if needed > self.stack.len() {
            self.stack.resize_with(needed * 2 + 1, ValueWord::none);
        }

        // Bind upvalue values as the first N locals
        for (i, upvalue) in upvalues.iter().enumerate() {
            if i < locals_count {
                self.stack[bp + i] = upvalue.get();
            }
        }

        // Bind the regular arguments after the upvalues
        for (i, arg) in args.iter().enumerate() {
            let local_idx = captures_count + i;
            if local_idx < locals_count {
                self.stack[bp + local_idx] = arg.clone();
            }
        }

        // Fill remaining parameters with None
        for i in (captures_count + args.len())..arity.min(locals_count) {
            self.stack[bp + i] = ValueWord::none();
        }

        self.sp = needed;

        let blob_hash = self.blob_hash_for_function(func_id);
        self.call_stack.push(CallFrame {
            return_ip: self.ip,
            base_pointer: bp,
            locals_count,
            function_id: Some(func_id),
            upvalues: Some(upvalues),
            blob_hash,
        });

        self.ip = entry_point;
        Ok(())
    }

    /// ValueWord-native call_value_immediate: dispatches on NanTag/HeapKind.
    ///
    /// Returns ValueWord directly.
    pub fn call_value_immediate_nb(
        &mut self,
        callee: &ValueWord,
        args: &[ValueWord],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::NanTag;
        let target_depth = self.call_stack.len();

        match callee.tag() {
            NanTag::Function => {
                let func_id = callee.as_function().ok_or(VMError::InvalidCall)?;
                self.call_function_with_nb_args(func_id, args)?;
            }
            NanTag::ModuleFunction => {
                let func_id = callee.as_module_function().ok_or(VMError::InvalidCall)?;
                let module_fn = self.module_fn_table.get(func_id).cloned().ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Module function ID {} not found in registry",
                        func_id
                    ))
                })?;
                let args_vec: Vec<ValueWord> = args.to_vec();
                let result_nb = self.invoke_module_fn(&module_fn, &args_vec)?;
                return Ok(result_nb);
            }
            NanTag::Heap => match callee.as_heap_ref() {
                Some(shape_value::HeapValue::Closure {
                    function_id,
                    upvalues,
                }) => {
                    self.call_closure_with_nb_args(*function_id, upvalues.clone(), args)?;
                }
                Some(shape_value::HeapValue::HostClosure(callable)) => {
                    let args_vec: Vec<ValueWord> = args.to_vec();
                    let result_nb = callable.call(&args_vec).map_err(VMError::RuntimeError)?;
                    return Ok(result_nb);
                }
                _ => return Err(VMError::InvalidCall),
            },
            _ => return Err(VMError::InvalidCall),
        }

        self.execute_until_call_depth(target_depth, ctx)?;
        self.pop_vw()
    }

    /// Fast-path function call: reads `arg_count` arguments directly from the
    /// value stack instead of collecting them into a temporary `Vec`.
    ///
    /// Precondition: the top `arg_count` values on the stack (below sp) are the
    /// arguments in left-to-right order (arg0 deepest, argN-1 at top).
    /// These args become the first locals of the new frame's register window.
    pub(crate) fn call_function_from_stack(
        &mut self,
        func_id: u16,
        arg_count: usize,
    ) -> Result<(), VMError> {
        let function = self
            .program
            .functions
            .get(func_id as usize)
            .ok_or(VMError::InvalidCall)?;

        if self.call_stack.len() >= self.config.max_call_depth {
            return Err(VMError::StackOverflow);
        }

        let locals_count = function.locals_count as usize;
        let entry_point = function.entry_point;
        let arity = function.arity as usize;

        // The args are already on the stack at positions [sp - arg_count .. sp).
        // They become the first locals in the register window.
        // bp = sp - arg_count (args are already in place as the first locals)
        let bp = self.sp.saturating_sub(arg_count);

        // Ensure stack has room for all locals (some may be beyond the args)
        let needed = bp + locals_count;
        if needed > self.stack.len() {
            self.stack.resize_with(needed * 2 + 1, ValueWord::none);
        }

        // Zero remaining local slots (including omitted args that the compiler
        // may intentionally represent as null sentinels for default params).
        let copy_count = arg_count.min(arity).min(locals_count);
        for i in copy_count..locals_count {
            self.stack[bp + i] = ValueWord::none();
        }

        // Advance sp past all locals
        self.sp = needed;

        let blob_hash = self.blob_hash_for_function(func_id);
        self.call_stack.push(CallFrame {
            return_ip: self.ip,
            base_pointer: bp,
            locals_count,
            function_id: Some(func_id),
            upvalues: None,
            blob_hash,
        });
        self.ip = entry_point;
        Ok(())
    }
}
