//! Native array builtin implementations
//!
//! Direct builtin methods — no string-based dispatch.

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt, heap_value::HeapValue};
use std::sync::Arc;

impl VirtualMachine {
    pub(in crate::executor) fn builtin_push(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "push() requires 2 arguments (array, value)".into(),
            ));
        }
        if let Some(view) = args[0].as_any_array() {
            let mut new_arr = view.to_generic().as_ref().clone();
            new_arr.push(args[1].clone());
            Ok(ValueWord::from_array(Arc::new(new_arr)))
        } else {
            Err(VMError::RuntimeError(
                "push() first argument must be an array".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_pop(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError(
                "pop() requires 1 argument (array)".into(),
            ));
        }
        if let Some(view) = args[0].as_any_array() {
            if view.is_empty() {
                return Ok(ValueWord::none());
            }
            let mut new_arr = view.to_generic().as_ref().clone();
            new_arr.pop();
            Ok(ValueWord::from_array(Arc::new(new_arr)))
        } else {
            Err(VMError::RuntimeError(
                "pop() argument must be an array".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_first(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("first() requires 1 argument".into()));
        }
        if let Some(view) = args[0].as_any_array() {
            Ok(view.first_nb().unwrap_or_else(ValueWord::none))
        } else {
            Err(VMError::RuntimeError(
                "first() argument must be an array".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_last(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("last() requires 1 argument".into()));
        }
        if let Some(view) = args[0].as_any_array() {
            Ok(view.last_nb().unwrap_or_else(ValueWord::none))
        } else {
            Err(VMError::RuntimeError(
                "last() argument must be an array".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_zip(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError("zip() requires 2 arguments".into()));
        }
        let a = args[0]
            .as_any_array()
            .ok_or_else(|| VMError::RuntimeError("zip() arguments must be arrays".into()))?
            .to_generic();
        let b = args[1]
            .as_any_array()
            .ok_or_else(|| VMError::RuntimeError("zip() arguments must be arrays".into()))?
            .to_generic();
        let result: Vec<ValueWord> = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| ValueWord::from_array(Arc::new(vec![x.clone(), y.clone()])))
            .collect();
        Ok(ValueWord::from_array(Arc::new(result)))
    }

    pub(in crate::executor) fn builtin_filled(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "Array.filled() requires 2 arguments (size, value)".into(),
            ));
        }
        let size = args[0]
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Array.filled() size must be a number".into()))?
            as usize;
        let value = args[1].clone();
        let array = vec![value; size];
        Ok(ValueWord::from_array(Arc::new(array)))
    }

    pub(in crate::executor) fn builtin_len(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("len() requires 1 argument".into()));
        }
        // Fast path: any array (generic + typed)
        if let Some(view) = args[0].as_any_array() {
            return Ok(ValueWord::from_i64(view.len() as i64));
        }
        // Fast path: string
        if let Some(s) = args[0].as_str() {
            return Ok(ValueWord::from_i64(s.len() as i64));
        }
        // TypedObject: return number of slots
        // cold-path: as_heap_ref retained — TypedObject slot count for len()
        if let Some(HeapValue::TypedObject { slots, .. }) = args[0].as_heap_ref() { // cold-path
            return Ok(ValueWord::from_i64(slots.len() as i64));
        }
        Err(VMError::RuntimeError(format!(
            "len() argument must be an array, string, or object, got {:?}",
            args[0].type_name()
        )))
    }
}
