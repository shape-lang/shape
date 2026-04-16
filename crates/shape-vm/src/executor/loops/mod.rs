//! Loop control operations for the VM executor
//!
//! Handles: LoopStart, LoopEnd, Break, Continue, IterNext, IterDone

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{LoopContext, VirtualMachine},
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_loops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            LoopStart => self.op_loop_start(instruction)?,
            LoopEnd => self.op_loop_end()?,
            Break => self.op_break()?,
            Continue => self.op_continue()?,
            IterNext => self.op_iter_next()?,
            IterDone => self.op_iter_done()?,
            _ => unreachable!(
                "exec_loops called with non-loop opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ===== Loop Control Operations =====

    pub(in crate::executor) fn op_loop_start(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(end_offset)) = instruction.operand {
            let end = (self.ip as i32 + end_offset) as usize;
            self.loop_stack.push(LoopContext {
                start: self.ip,
                end,
            });

            // OSR: record back-edge and attempt entry into JIT-compiled loop.
            // The LoopStart IP (self.ip - 1 since we already advanced past it)
            // is the canonical loop header IP used as the key.
            #[cfg(feature = "jit")]
            {
                let loop_ip = self.ip.saturating_sub(1);
                if let Some(func_id) = self.current_function_id() {
                    // Record the back-edge iteration; request compilation if hot
                    self.check_osr_back_edge(func_id, loop_ip);

                    // Attempt OSR entry if compiled code is available
                    if self.try_osr_entry(func_id, loop_ip)? {
                        // OSR succeeded: IP is set to loop exit, pop the loop
                        // context we just pushed (the JIT executed the loop).
                        self.loop_stack.pop();
                        return Ok(());
                    }
                }
            }
        }
        // Without operand, LoopStart is a JIT-only marker (no-op for interpreter).
        // Break/continue are compiled as Jump instructions, not Break/Continue opcodes.
        Ok(())
    }

    pub(in crate::executor) fn op_loop_end(&mut self) -> Result<(), VMError> {
        self.loop_stack.pop();
        Ok(())
    }

    pub(in crate::executor) fn op_break(&mut self) -> Result<(), VMError> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.ip = loop_ctx.end;
            Ok(())
        } else {
            Err(VMError::RuntimeError("break outside of loop".to_string()))
        }
    }

    pub(in crate::executor) fn op_continue(&mut self) -> Result<(), VMError> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.ip = loop_ctx.start;
            Ok(())
        } else {
            Err(VMError::RuntimeError(
                "continue outside of loop".to_string(),
            ))
        }
    }

    pub(in crate::executor) fn op_iter_done(&mut self) -> Result<(), VMError> {
        let idx_nb = self.pop_raw_u64()?;
        let iter = self.pop_raw_u64()?;
        let idx = idx_nb.as_number_coerce().ok_or(VMError::TypeError {
            expected: "number",
            got: "unknown",
        })? as i64;
        // v2 typed array fast path: read len directly from the stamped header.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(&iter)
        {
            let done = idx < 0 || idx as u32 >= view.len;
            self.push_raw_bool(done)?;
            return Ok(());
        }
        // Handle unified arrays (bit-47 tagged) for iteration.
        if shape_value::tags::is_unified_heap(iter.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(iter.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(iter.raw_bits())
                };
                let done = idx < 0 || idx as usize >= arr.len();
                self.push_raw_bool(done)?;
                return Ok(());
            }
        }
        // cold-path: as_heap_ref retained — multi-variant iteration done check
        let done = match iter.as_heap_ref() { // cold-path
            Some(HeapValue::Array(arr)) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::IntArray(arr)) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::FloatArray(arr)) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::FloatArraySlice { len, .. }) => idx < 0 || idx as usize >= *len as usize,
            Some(HeapValue::BoolArray(arr)) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::String(s)) => idx < 0 || idx as usize >= s.len(),
            Some(HeapValue::Range {
                start,
                end,
                inclusive,
            }) => {
                let start_val = start
                    .as_ref()
                    .and_then(|s| s.as_number_coerce())
                    .unwrap_or(0.0) as i64;
                let end_val = end
                    .as_ref()
                    .and_then(|e| e.as_number_coerce())
                    .unwrap_or(i64::MAX as f64) as i64;
                let end_val = if *inclusive { end_val + 1 } else { end_val };
                let count = end_val - start_val;
                count <= 0 || idx >= count
            }
            Some(HeapValue::DataTable(dt)) => idx < 0 || idx as usize >= dt.row_count(),
            Some(HeapValue::TypedTable { table, .. }) => {
                idx < 0 || idx as usize >= table.row_count()
            }
            Some(HeapValue::Iterator(state)) => {
                // For iterators without transforms, delegate to source length.
                // Iterators with transforms should be .collect()'d before for-loop.
                if state.done {
                    true
                } else {
                    let src_len =
                        crate::executor::objects::iterator_methods::iter_source_len(&state.source);
                    idx < 0 || idx as usize >= src_len
                }
            }
            Some(HeapValue::HashMap(hm)) => idx < 0 || idx as usize >= hm.keys.len(),
            _ => {
                return Err(VMError::TypeError {
                    expected: "array, string, range, table, iterator, or hashmap",
                    got: iter.type_name(),
                });
            }
        };
        self.push_raw_bool(done)?;
        Ok(())
    }

    pub(in crate::executor) fn op_iter_next(&mut self) -> Result<(), VMError> {
        let idx_nb = self.pop_raw_u64()?;
        let iter = self.pop_raw_u64()?;
        let idx = idx_nb.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("Expected number for iterator index".to_string())
        })? as i64;
        // v2 typed array fast path: read element through the stamped header.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(&iter)
        {
            let result = if idx < 0 || idx as u32 >= view.len {
                ValueWord::none()
            } else {
                crate::executor::v2_handlers::v2_array_detect::read_element(&view, idx as u32)
                    .unwrap_or_else(ValueWord::none)
            };
            self.push_raw_u64(result)?;
            return Ok(());
        }
        // Handle unified arrays (bit-47 tagged) for iteration.
        if shape_value::tags::is_unified_heap(iter.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(iter.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(iter.raw_bits())
                };
                let result = if idx < 0 || idx as usize >= arr.len() {
                    ValueWord::none()
                } else {
                    let elem_bits = *arr.get(idx as usize).unwrap();
                    unsafe { ValueWord::clone_from_bits(elem_bits) }
                };
                self.push_raw_u64(result)?;
                return Ok(());
            }
        }
        // cold-path: as_heap_ref retained — multi-variant iteration next element
        let result = match iter.as_heap_ref() { // cold-path
            Some(HeapValue::Array(arr)) => {
                if idx < 0 {
                    ValueWord::none()
                } else {
                    arr.get(idx as usize)
                        .cloned()
                        .unwrap_or_else(ValueWord::none)
                }
            }
            Some(HeapValue::IntArray(arr)) => {
                if idx < 0 || idx as usize >= arr.len() {
                    ValueWord::none()
                } else {
                    ValueWord::from_i64(arr[idx as usize])
                }
            }
            Some(HeapValue::FloatArray(arr)) => {
                if idx < 0 || idx as usize >= arr.len() {
                    ValueWord::none()
                } else {
                    ValueWord::from_f64(arr[idx as usize])
                }
            }
            Some(HeapValue::FloatArraySlice { parent, offset, len }) => {
                let slice_len = *len as usize;
                if idx < 0 || idx as usize >= slice_len {
                    ValueWord::none()
                } else {
                    let off = *offset as usize;
                    ValueWord::from_f64(parent.data[off + idx as usize])
                }
            }
            Some(HeapValue::BoolArray(arr)) => {
                if idx < 0 || idx as usize >= arr.len() {
                    ValueWord::none()
                } else {
                    ValueWord::from_bool(arr[idx as usize] != 0)
                }
            }
            Some(HeapValue::String(s)) => {
                if idx < 0 {
                    ValueWord::none()
                } else {
                    s.chars()
                        .nth(idx as usize)
                        .map(ValueWord::from_char)
                        .unwrap_or_else(ValueWord::none)
                }
            }
            Some(HeapValue::Range {
                start,
                end,
                inclusive,
            }) => {
                let start_val = start
                    .as_ref()
                    .and_then(|s| s.as_number_coerce())
                    .unwrap_or(0.0) as i64;
                let end_val = end
                    .as_ref()
                    .and_then(|e| e.as_number_coerce())
                    .unwrap_or(i64::MAX as f64) as i64;
                let end_val = if *inclusive { end_val + 1 } else { end_val };
                let count = end_val - start_val;
                if count <= 0 || idx < 0 || idx >= count {
                    ValueWord::none()
                } else {
                    ValueWord::from_i64(start_val + idx)
                }
            }
            Some(HeapValue::DataTable(dt)) => {
                if idx < 0 || idx as usize >= dt.row_count() {
                    ValueWord::none()
                } else {
                    ValueWord::from_row_view(0, dt.clone(), idx as usize)
                }
            }
            Some(HeapValue::TypedTable { schema_id, table }) => {
                if idx < 0 || idx as usize >= table.row_count() {
                    ValueWord::none()
                } else {
                    ValueWord::from_row_view(*schema_id, table.clone(), idx as usize)
                }
            }
            Some(HeapValue::Iterator(state)) => {
                // For iterators in for-loops, index into the source directly.
                crate::executor::objects::iterator_methods::iter_source_element_at(
                    &state.source,
                    idx as usize,
                )
                .unwrap_or_else(ValueWord::none)
            }
            Some(HeapValue::HashMap(hm)) => {
                if idx < 0 || idx as usize >= hm.keys.len() {
                    ValueWord::none()
                } else {
                    let i = idx as usize;
                    ValueWord::from_array(Arc::new(vec![hm.keys[i].clone(), hm.values[i].clone()]))
                }
            }
            _ => {
                return Err(VMError::TypeError {
                    expected: "array, string, range, table, iterator, or hashmap",
                    got: iter.type_name(),
                });
            }
        };
        self.push_raw_u64(result)?;
        Ok(())
    }
}
