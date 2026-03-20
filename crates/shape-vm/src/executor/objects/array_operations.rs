//! Array operations (ArrayPush, ArrayPushLocal, ArrayPop, SliceAccess)
//!
//! Handles array manipulation and slicing for arrays, series, and strings.

use crate::executor::VirtualMachine;
use shape_value::nanboxed::RefTarget;
use shape_value::{HeapValue, VMError, ValueWord};
use std::sync::Arc;

impl VirtualMachine {
    pub(in crate::executor) fn op_array_push(&mut self) -> Result<(), VMError> {
        let value_nb = self.pop_vw()?;
        let mut array_nb = self.pop_vw()?;

        // Handle unified arrays (bit-47 tagged) for push.
        if shape_value::tags::is_unified_heap(array_nb.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(array_nb.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits_mut(array_nb.raw_bits())
                };
                let new_bits = value_nb.raw_bits();
                std::mem::forget(value_nb);
                arr.push(new_bits);
                self.push_vw(array_nb)?;
                return Ok(());
            }
        }

        // Try mutable access for any array variant
        if let Some(mut view) = array_nb.as_any_array_mut() {
            match &mut view {
                shape_value::ArrayViewMut::Generic(arc_vec) => {
                    Arc::make_mut(arc_vec).push(value_nb);
                    self.push_vw(array_nb)?;
                    return Ok(());
                }
                shape_value::ArrayViewMut::Int(arc_vec) => {
                    if let Some(i) = value_nb.as_i64() {
                        Arc::make_mut(arc_vec).push(i);
                        self.push_vw(array_nb)?;
                        return Ok(());
                    }
                    let generic = array_nb.as_any_array().unwrap().to_generic();
                    let mut vec = (*generic).clone();
                    vec.push(value_nb);
                    self.push_vw(ValueWord::from_array(Arc::new(vec)))?;
                    return Ok(());
                }
                shape_value::ArrayViewMut::Float(arc_vec) => {
                    if let Some(f) = value_nb.as_f64() {
                        Arc::make_mut(arc_vec).push(f);
                        self.push_vw(array_nb)?;
                        return Ok(());
                    }
                    let generic = array_nb.as_any_array().unwrap().to_generic();
                    let mut vec = (*generic).clone();
                    vec.push(value_nb);
                    self.push_vw(ValueWord::from_array(Arc::new(vec)))?;
                    return Ok(());
                }
                shape_value::ArrayViewMut::Bool(arc_vec) => {
                    if let Some(b) = value_nb.as_bool() {
                        Arc::make_mut(arc_vec).push(if b { 1 } else { 0 });
                        self.push_vw(array_nb)?;
                        return Ok(());
                    }
                    let generic = array_nb.as_any_array().unwrap().to_generic();
                    let mut vec = (*generic).clone();
                    vec.push(value_nb);
                    self.push_vw(ValueWord::from_array(Arc::new(vec)))?;
                    return Ok(());
                }
            }
        }

        Err(VMError::TypeError {
            expected: "array",
            got: array_nb.type_name(),
        })
    }

    /// Push a value into an array stored in a local or module_binding variable slot,
    /// mutating in-place. Bypasses Arc reconstruction overhead by directly
    /// accessing the heap pointer from the ValueWord bits.
    ///
    /// This turns O(n^2) array construction (from repeated clone+push) into O(n).
    pub(in crate::executor) fn op_array_push_local(
        &mut self,
        instruction: &crate::bytecode::Instruction,
    ) -> Result<(), VMError> {
        use crate::bytecode::Operand;
        let value_nb = self.pop_vw()?;

        match instruction.operand {
            Some(Operand::Local(idx)) => {
                let bp = self.current_locals_base();
                let slot = bp + idx as usize;
                match self.stack[slot].as_ref_target() {
                    Some(RefTarget::Stack(target)) => {
                        Self::push_to_array_slot(&mut self.stack[target], value_nb)
                    }
                    Some(RefTarget::ModuleBinding(target)) => {
                        if target >= self.module_bindings.len() {
                            return Err(VMError::RuntimeError(format!(
                                "ModuleBinding index {} out of bounds",
                                target
                            )));
                        }
                        Self::push_to_array_slot(&mut self.module_bindings[target], value_nb)
                    }
                    Some(target) => {
                        let mut array_nb = self.read_ref_target(&target)?;
                        Self::push_to_array_slot(&mut array_nb, value_nb)?;
                        self.write_ref_target(&target, array_nb)
                    }
                    None => Self::push_to_array_slot(&mut self.stack[slot], value_nb),
                }
            }
            Some(Operand::ModuleBinding(idx)) => {
                let slot = idx as usize;
                if slot >= self.module_bindings.len() {
                    return Err(VMError::RuntimeError(format!(
                        "ModuleBinding index {} out of bounds",
                        slot
                    )));
                }
                match self.module_bindings[slot].as_ref_target() {
                    Some(RefTarget::Stack(target)) => {
                        Self::push_to_array_slot(&mut self.stack[target], value_nb)
                    }
                    Some(RefTarget::ModuleBinding(target)) => {
                        if target >= self.module_bindings.len() {
                            return Err(VMError::RuntimeError(format!(
                                "ModuleBinding index {} out of bounds",
                                target
                            )));
                        }
                        Self::push_to_array_slot(&mut self.module_bindings[target], value_nb)
                    }
                    Some(target) => {
                        let mut array_nb = self.read_ref_target(&target)?;
                        Self::push_to_array_slot(&mut array_nb, value_nb)?;
                        self.write_ref_target(&target, array_nb)
                    }
                    None => Self::push_to_array_slot(&mut self.module_bindings[slot], value_nb),
                }
            }
            _ => Err(VMError::RuntimeError(
                "ArrayPushLocal requires Local or ModuleBinding operand".into(),
            )),
        }
    }

    /// Push a value to an array stored at a ValueWord slot, mutating in-place.
    /// Directly accesses the heap pointer to avoid Arc reconstruction overhead.
    #[inline(always)]
    fn push_to_array_slot(slot: &mut ValueWord, value: ValueWord) -> Result<(), VMError> {
        const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;
        const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

        let bits = slot.raw_bits();
        let is_tagged = (bits & TAG_BASE) == TAG_BASE;
        let tag = (bits >> 48) & 0x7;

        // Handle unified arrays (bit-47 tagged).
        if is_tagged && tag == 0 && shape_value::tags::is_unified_heap(bits) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(bits) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits_mut(bits)
                };
                let new_bits = value.raw_bits();
                std::mem::forget(value);
                arr.push(new_bits);
                return Ok(());
            }
        }

        if is_tagged && tag == 0 {
            let ptr = (bits & PAYLOAD_MASK) as *mut HeapValue;
            if !ptr.is_null() {
                let heap_val = unsafe { &mut *ptr };
                match heap_val {
                    HeapValue::Array(arc_vec) => {
                        // BARRIER: heap write site — appends value to generic array (may contain heap pointer)
                        if let Some(vec) = Arc::get_mut(arc_vec) {
                            vec.push(value);
                            return Ok(());
                        }
                        Arc::make_mut(arc_vec).push(value);
                        return Ok(());
                    }
                    HeapValue::IntArray(arc_vec) => {
                        if let Some(i) = value.as_i64() {
                            Arc::make_mut(arc_vec).push(i);
                            return Ok(());
                        }
                        let len = arc_vec.len();
                        let mut generic: Vec<ValueWord> = Vec::with_capacity(len + 1);
                        for &v in arc_vec.iter() {
                            generic.push(ValueWord::from_i64(v));
                        }
                        generic.push(value);
                        *heap_val = HeapValue::Array(Arc::new(generic));
                        return Ok(());
                    }
                    HeapValue::FloatArray(arc_vec) => {
                        if let Some(f) = value.as_f64() {
                            Arc::make_mut(arc_vec).push(f);
                            return Ok(());
                        }
                        let len = arc_vec.len();
                        let mut generic: Vec<ValueWord> = Vec::with_capacity(len + 1);
                        for &v in arc_vec.iter() {
                            generic.push(ValueWord::from_f64(v));
                        }
                        generic.push(value);
                        *heap_val = HeapValue::Array(Arc::new(generic));
                        return Ok(());
                    }
                    HeapValue::BoolArray(arc_vec) => {
                        if let Some(b) = value.as_bool() {
                            Arc::make_mut(arc_vec).push(if b { 1 } else { 0 });
                            return Ok(());
                        }
                        let len = arc_vec.len();
                        let mut generic: Vec<ValueWord> = Vec::with_capacity(len + 1);
                        for &v in arc_vec.iter() {
                            generic.push(ValueWord::from_bool(v != 0));
                        }
                        generic.push(value);
                        *heap_val = HeapValue::Array(Arc::new(generic));
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        Err(VMError::TypeError {
            expected: "array",
            got: "unknown",
        })
    }

    pub(in crate::executor) fn op_array_pop(&mut self) -> Result<(), VMError> {
        let array_nb = self.pop_vw()?;

        let arr = array_nb.as_any_array().ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: array_nb.type_name(),
        })?;

        let value = arr.last_nb().unwrap_or_else(ValueWord::none);
        self.push_vw(value)?;
        Ok(())
    }

    pub(in crate::executor) fn op_slice_access(&mut self) -> Result<(), VMError> {
        let end_nb = self.pop_vw()?;
        let start_nb = self.pop_vw()?;
        let array_nb = self.pop_vw()?;

        let end = end_nb.as_number_coerce().ok_or(VMError::TypeError {
            expected: "number",
            got: "unknown",
        })? as i32;
        let start = start_nb.as_number_coerce().ok_or(VMError::TypeError {
            expected: "number",
            got: "unknown",
        })? as i32;

        // Handle unified arrays (bit-47 tagged) for slice access.
        if shape_value::tags::is_unified_heap(array_nb.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(array_nb.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(array_nb.raw_bits())
                };
                let len = arr.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(arr.len())
                };
                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < arr.len()
                {
                    (actual_start..actual_end)
                        .map(|i| unsafe { ValueWord::clone_from_bits(*arr.get(i).unwrap()) })
                        .collect()
                } else {
                    Vec::new()
                };
                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
                return Ok(());
            }
        }

        use shape_value::heap_value::HeapValue;
        match array_nb.as_heap_ref() {
            Some(HeapValue::Array(arr)) => {
                let len = arr.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(arr.len())
                };

                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < arr.len()
                {
                    arr[actual_start..actual_end].to_vec()
                } else {
                    Vec::new()
                };

                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
            }
            Some(HeapValue::IntArray(arr)) => {
                let len = arr.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(arr.len())
                };

                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < arr.len()
                {
                    arr[actual_start..actual_end]
                        .iter()
                        .map(|&v| ValueWord::from_i64(v))
                        .collect()
                } else {
                    Vec::new()
                };

                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
            }
            Some(HeapValue::FloatArray(arr)) => {
                let len = arr.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(arr.len())
                };

                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < arr.len()
                {
                    arr[actual_start..actual_end]
                        .iter()
                        .map(|&v| ValueWord::from_f64(v))
                        .collect()
                } else {
                    Vec::new()
                };

                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
            }
            Some(HeapValue::FloatArraySlice { parent, offset, len: slice_len }) => {
                let total = *slice_len as usize;
                let off = *offset as usize;
                let data = &parent.data[off..off + total];
                let len_i32 = total as i32;
                let actual_start = if start < 0 {
                    (len_i32 + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len_i32 + end).max(0) as usize
                } else {
                    (end as usize).min(total)
                };

                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < total
                {
                    data[actual_start..actual_end]
                        .iter()
                        .map(|&v| ValueWord::from_f64(v))
                        .collect()
                } else {
                    Vec::new()
                };

                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
            }
            Some(HeapValue::BoolArray(arr)) => {
                let len = arr.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(arr.len())
                };

                let slice: Vec<ValueWord> = if actual_start < actual_end && actual_start < arr.len()
                {
                    arr[actual_start..actual_end]
                        .iter()
                        .map(|&v| ValueWord::from_bool(v != 0))
                        .collect()
                } else {
                    Vec::new()
                };

                self.push_vw(ValueWord::from_array(Arc::new(slice)))?;
            }
            Some(HeapValue::String(s)) => {
                let len = s.len() as i32;
                let actual_start = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let actual_end = if end < 0 {
                    (len + end).max(0) as usize
                } else {
                    (end as usize).min(s.len())
                };

                let slice_str = if actual_start < actual_end && actual_start < s.len() {
                    s[actual_start..actual_end].to_string()
                } else {
                    String::new()
                };

                self.push_vw(ValueWord::from_string(Arc::new(slice_str)))?;
            }
            _ => {
                return Err(VMError::TypeError {
                    expected: "array or string",
                    got: array_nb.type_name(),
                });
            }
        }
        Ok(())
    }
}
