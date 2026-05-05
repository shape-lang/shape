//! Loop control operations for the VM executor
//!
//! Handles: LoopStart, LoopEnd, Break, Continue, IterNext, IterDone

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{LoopContext, VirtualMachine},
};
use shape_value::heap_value::HeapValue;
use shape_value::tag_bits::{get_payload, get_tag, is_tagged, sign_extend_i48, TAG_INT};
use shape_value::{TypedArrayData, TableViewData, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Decode a loop iterator-protocol index from raw stack bits.
///
/// E+5.5: typed locals and `AddInt`/`PushConst Int` push native i64 raw bits
/// (no NaN-tag). The legacy `as_number_coerce()` decode path interprets
/// untagged bits as f64, which silently mangles small native i64 values
/// (e.g., raw bits 1 → subnormal f64 ≈ 5e-324 → cast to i64 = 0), causing
/// for-loops to never advance. This decoder handles all three encodings the
/// idx slot may carry:
///   - Tagged i48 (`TAG_INT`): legacy compiler emit sites that haven't
///     migrated to native i64 yet.
///   - Untagged subnormal-or-zero with non-zero raw bits: native i64
///     (post-E+5.5 typed slot or `AddInt` result).
///   - Untagged normal f64 (or canonical zero): real f64 value (test
///     harness using `Constant::Number(N.0)` for the idx).
#[inline(always)]
fn decode_iter_idx(bits: u64) -> Result<i64, VMError> {
    if is_tagged(bits) {
        if get_tag(bits) == TAG_INT {
            return Ok(sign_extend_i48(get_payload(bits)));
        }
        return Err(VMError::TypeError {
            expected: "number",
            got: "unknown",
        });
    }
    // Untagged: distinguish native i64 from f64. f64::from_bits(N) for
    // 1 <= N < 2^52 is always subnormal (exponent bits = 0), so any
    // subnormal-with-nonzero-bits is unambiguously native i64.
    let f = f64::from_bits(bits);
    if f.is_subnormal() && bits != 0 {
        Ok(bits as i64)
    } else {
        Ok(f as i64)
    }
}

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
        let idx_bits = self.pop_raw_u64()?;
        let iter = self.pop_raw_u64()?;
        let idx = decode_iter_idx(idx_bits)?;
        // v2 typed array fast path: read len directly from the stamped header.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(&iter)
        {
            let done = idx < 0 || idx as u32 >= view.len;
            self.push_tagged_bool(done)?;
            return Ok(());
        }
        // Handle unified arrays (bit-47 tagged) for iteration.
        let vb = shape_value::ValueBits::from_raw(iter.raw_bits());
        if vb.is_unified_heap() {
            let kind = unsafe { vb.unified_heap_kind() };
            if kind == shape_value::tag_bits::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(iter.raw_bits())
                };
                let done = idx < 0 || idx as usize >= arr.len();
                self.push_tagged_bool(done)?;
                return Ok(());
            }
        }
        // cold-path: as_heap_ref retained — multi-variant iteration done check
        let done = match iter.as_heap_ref() { // cold-path
            Some(HeapValue::Array(arr)) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::TypedArray(TypedArrayData::I64(arr))) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::TypedArray(TypedArrayData::F64(arr))) => idx < 0 || idx as usize >= arr.len(),
            Some(HeapValue::TypedArray(TypedArrayData::FloatSlice { len, .. })) => idx < 0 || idx as usize >= *len as usize,
            Some(HeapValue::TypedArray(TypedArrayData::Bool(arr))) => idx < 0 || idx as usize >= arr.len(),
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
            Some(HeapValue::TableView(TableViewData::TypedTable { table, .. })) => {
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
        self.push_tagged_bool(done)?;
        Ok(())
    }

    pub(in crate::executor) fn op_iter_next(&mut self) -> Result<(), VMError> {
        let idx_bits = self.pop_raw_u64()?;
        let iter = self.pop_raw_u64()?;
        let idx = decode_iter_idx(idx_bits)?;
        // v2 typed array fast path: read element through the stamped header.
        //
        // Wave E+5.5 producer-side flip: for primitive native kinds (I64,
        // I32, F64, Bool) push raw native bits, matching the typed
        // `LoadLocalI64`/`LoadLocalF64`/`LoadLocalBool` consumers emitted
        // for for-loop variables whose `set_local_type_info` resolves to
        // a native primitive (`compiler/loops.rs:411`). The legacy tagged
        // `ValueWord::from_i64` push was observable as `pop_native_i64`
        // reading a tagged-i48 bit pattern as a huge negative i64 — e.g.
        // `MulInt(1, x)` returning ~-5e33 for `x=2` instead of 2.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(&iter)
        {
            use crate::executor::v2_handlers::v2_array_detect::V2ElemType;
            use shape_value::v2::typed_array::TypedArray;
            if idx < 0 || idx as u32 >= view.len {
                self.push_raw_u64(ValueWord::none())?;
                return Ok(());
            }
            let i = idx as u32;
            match view.elem_type {
                V2ElemType::I64 => {
                    let arr = view.ptr as *const TypedArray<i64>;
                    self.push_native_i64(unsafe { TypedArray::<i64>::get_unchecked(arr, i) })?;
                }
                V2ElemType::I32 => {
                    let arr = view.ptr as *const TypedArray<i32>;
                    let val = unsafe { TypedArray::<i32>::get_unchecked(arr, i) };
                    self.push_native_i64(val as i64)?;
                }
                V2ElemType::F64 => {
                    let arr = view.ptr as *const TypedArray<f64>;
                    self.push_raw_f64(unsafe { TypedArray::<f64>::get_unchecked(arr, i) })?;
                }
                V2ElemType::Bool => {
                    let arr = view.ptr as *const TypedArray<u8>;
                    let val = unsafe { TypedArray::<u8>::get_unchecked(arr, i) };
                    self.push_native_bool(val != 0)?;
                }
            }
            return Ok(());
        }
        // Handle unified arrays (bit-47 tagged) for iteration.
        let vb = shape_value::ValueBits::from_raw(iter.raw_bits());
        if vb.is_unified_heap() {
            let kind = unsafe { vb.unified_heap_kind() };
            if kind == shape_value::tag_bits::HEAP_KIND_ARRAY as u16 {
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
            Some(HeapValue::TypedArray(TypedArrayData::I64(arr))) => {
                // Wave E+5.5 producer-side flip: push raw native i64 bits
                // (no tag), matching the typed `LoadLocalI64` consumer
                // emitted for loop vars whose static element type is `int`.
                if idx < 0 || idx as usize >= arr.len() {
                    self.push_raw_u64(ValueWord::none())?;
                } else {
                    self.push_native_i64(arr[idx as usize])?;
                }
                return Ok(());
            }
            Some(HeapValue::TypedArray(TypedArrayData::F64(arr))) => {
                // `ValueWord::from_f64` produces raw f64 bits with no
                // NaN-tag (`value_word_ext.rs:463`), matching the typed
                // `LoadLocalF64` raw-bit read. Use `push_raw_f64`.
                if idx < 0 || idx as usize >= arr.len() {
                    self.push_raw_u64(ValueWord::none())?;
                } else {
                    self.push_raw_f64(arr[idx as usize])?;
                }
                return Ok(());
            }
            Some(HeapValue::TypedArray(TypedArrayData::FloatSlice { parent, offset, len })) => {
                let slice_len = *len as usize;
                if idx < 0 || idx as usize >= slice_len {
                    self.push_raw_u64(ValueWord::none())?;
                } else {
                    let off = *offset as usize;
                    self.push_raw_f64(parent.data[off + idx as usize])?;
                }
                return Ok(());
            }
            Some(HeapValue::TypedArray(TypedArrayData::Bool(arr))) => {
                // Native bool bits (0u64 / 1u64) per the typed-Bool
                // `LoadLocalBool` raw-bit consumer contract.
                if idx < 0 || idx as usize >= arr.len() {
                    self.push_raw_u64(ValueWord::none())?;
                } else {
                    self.push_native_bool(arr[idx as usize] != 0)?;
                }
                return Ok(());
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
                // Wave E+5.5 producer-side flip: Range iteration yields
                // i64 elements; push raw native bits to match the typed
                // `LoadLocalI64` consumer emitted for `for i in 0..n` loop
                // vars (whose `iter_element_type_name` resolves to `int`).
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
                    self.push_raw_u64(ValueWord::none())?;
                } else {
                    self.push_native_i64(start_val + idx)?;
                }
                return Ok(());
            }
            Some(HeapValue::DataTable(dt)) => {
                if idx < 0 || idx as usize >= dt.row_count() {
                    ValueWord::none()
                } else {
                    ValueWord::from_row_view(0, dt.clone(), idx as usize)
                }
            }
            Some(HeapValue::TableView(TableViewData::TypedTable { schema_id, table })) => {
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
                    ValueWord::from_array(shape_value::vmarray_from_vec(vec![hm.keys[i].clone(), hm.values[i].clone()]))
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
