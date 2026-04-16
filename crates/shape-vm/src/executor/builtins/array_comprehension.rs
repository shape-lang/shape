//! Array comprehension builtin implementations
//!
//! Higher-order functions: map, filter, reduce, forEach, find, findIndex, some, every

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{HeapKind, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Check that a ValueWord value is callable
#[inline]
fn is_callable(nb: &ValueWord) -> bool {
    nb.is_function() || nb.is_module_function() || (nb.is_heap() && matches!(
        nb.heap_kind(),
        Some(HeapKind::Closure | HeapKind::HostClosure)
    ))
}

/// Extract bool from ValueWord call result
#[inline]
fn as_bool_result(nb: &ValueWord, fn_name: &str) -> Result<bool, VMError> {
    nb.as_bool().ok_or_else(|| {
        VMError::RuntimeError(format!("{}() predicate must return boolean", fn_name))
    })
}

impl VirtualMachine {
    /// Map: Transform array elements via callback function
    pub(in crate::executor) fn builtin_map(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "map() requires exactly 2 arguments (array, function)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("map() first argument must be an array".to_string())
            })?
            .to_generic();

        if !is_callable(&args[1]) {
            return Err(VMError::RuntimeError(
                "map() second argument must be a function".to_string(),
            ));
        }
        // Check arity for plain functions
        if let Some(func_id) = args[1].as_function_id() {
            let function = self
                .program
                .functions
                .get(func_id as usize)
                .ok_or(VMError::InvalidCall)?;
            if function.arity != 1 {
                return Err(VMError::RuntimeError(
                    "map() callback function must take exactly 1 parameter".to_string(),
                ));
            }
        }

        let mut results = Vec::with_capacity(array.len());
        for nb in array.iter() {
            let mapped =
                self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            results.push(mapped);
        }

        Ok(ValueWord::from_array(Arc::new(results)))
    }

    /// Filter: Keep array elements matching predicate
    pub(in crate::executor) fn builtin_filter(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "filter() requires exactly 2 arguments (array, predicate)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("filter() first argument must be an array".to_string())
            })?
            .to_generic();

        if !is_callable(&args[1]) {
            return Err(VMError::RuntimeError(
                "filter() second argument must be a function".to_string(),
            ));
        }

        let mut filtered: Vec<ValueWord> = Vec::new();
        for nb in array.iter() {
            let keep = self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if as_bool_result(&keep, "filter")? {
                filtered.push(nb.clone());
            }
        }

        Ok(ValueWord::from_array(Arc::new(filtered)))
    }

    /// Reduce: Fold array to single value
    pub(in crate::executor) fn builtin_reduce(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(VMError::RuntimeError(
                "reduce() requires 2-3 arguments (array, reducer, [initial])".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("reduce() first argument must be an array".to_string())
            })?
            .to_generic();

        let mut accumulator = if args.len() == 3 {
            args[2].clone()
        } else {
            if array.is_empty() {
                return Err(VMError::RuntimeError(
                    "reduce() requires initial value for empty array".to_string(),
                ));
            }
            array[0].clone()
        };

        let start_idx = if args.len() == 3 { 0 } else { 1 };
        for nb in array.iter().skip(start_idx) {
            accumulator = self.call_value_immediate_nb(
                &args[1],
                &[accumulator, nb.clone()],
                ctx.as_deref_mut(),
            )?;
        }

        Ok(accumulator)
    }

    /// ForEach: Execute function for each element (side effects)
    pub(in crate::executor) fn builtin_for_each(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "forEach() requires exactly 2 arguments (array, function)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("forEach() first argument must be an array".to_string())
            })?
            .to_generic();

        for nb in array.iter() {
            self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        }

        Ok(ValueWord::none())
    }

    /// Find: Return first element matching predicate
    pub(in crate::executor) fn builtin_find(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "find() requires exactly 2 arguments (array, predicate)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("find() first argument must be an array".to_string())
            })?
            .to_generic();

        for nb in array.iter() {
            let matches =
                self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if as_bool_result(&matches, "find")? {
                return Ok(nb.clone());
            }
        }

        Ok(ValueWord::none())
    }

    /// FindIndex: Return index of first element matching predicate
    pub(in crate::executor) fn builtin_find_index(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "findIndex() requires exactly 2 arguments (array, predicate)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("findIndex() first argument must be an array".to_string())
            })?
            .to_generic();

        for (index, nb) in array.iter().enumerate() {
            let matches =
                self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if as_bool_result(&matches, "findIndex")? {
                return Ok(ValueWord::from_f64(index as f64));
            }
        }

        Ok(ValueWord::from_f64(-1.0))
    }

    /// Some: Check if any element matches predicate
    pub(in crate::executor) fn builtin_some(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "some() requires exactly 2 arguments (array, predicate)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("some() first argument must be an array".to_string())
            })?
            .to_generic();

        for nb in array.iter() {
            let matches =
                self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if as_bool_result(&matches, "some")? {
                return Ok(ValueWord::from_bool(true));
            }
        }

        Ok(ValueWord::from_bool(false))
    }

    /// Every: Check if all elements match predicate
    pub(in crate::executor) fn builtin_every(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "every() requires exactly 2 arguments (array, predicate)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("every() first argument must be an array".to_string())
            })?
            .to_generic();

        for nb in array.iter() {
            let matches =
                self.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if !as_bool_result(&matches, "every")? {
                return Ok(ValueWord::from_bool(false));
            }
        }

        Ok(ValueWord::from_bool(true))
    }
}
