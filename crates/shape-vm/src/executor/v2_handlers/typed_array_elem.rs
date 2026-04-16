//! Typed array element access opcodes (local-slot based).
//!
//! These opcodes skip the HeapValue enum dispatch when the compiler proves the
//! element type.  The array lives in a local slot (Operand::Local) as a NaN-boxed
//! `HeapValue::TypedArray(TypedArrayData::I64|F64|...)`.  The index (and value
//! for set/push) are on the operand stack.
//!
//! ## Opcodes handled here
//!
//! | Opcode        | Stack in         | Stack out | Operand      |
//! |---------------|------------------|-----------|--------------|
//! | GetElemI64    | [index]          | [value]   | Local(slot)  |
//! | GetElemF64    | [index]          | [value]   | Local(slot)  |
//! | SetElemI64    | [index, value]   | []        | Local(slot)  |
//! | SetElemF64    | [index, value]   | []        | Local(slot)  |
//! | ArrayPushI64  | [value]          | []        | Local(slot)  |
//! | ArrayPushF64  | [value]          | []        | Local(slot)  |
//! | ArrayLenTyped | []               | [len]     | Local(slot)  |

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::heap_value::TypedArrayData;
use shape_value::{HeapValue, VMError, ValueWord, ValueWordExt};

use super::super::VirtualMachine;

impl VirtualMachine {
    /// Dispatch for the typed array element access opcodes.
    pub(crate) fn exec_typed_array_elem_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::GetElemI64 => self.op_get_elem_i64(instruction),
            OpCode::GetElemF64 => self.op_get_elem_f64(instruction),
            OpCode::SetElemI64 => self.op_set_elem_i64(instruction),
            OpCode::SetElemF64 => self.op_set_elem_f64(instruction),
            OpCode::ArrayPushI64 => self.op_array_push_i64_elem(instruction),
            OpCode::ArrayPushF64 => self.op_array_push_f64_elem(instruction),
            OpCode::ArrayLenTyped => self.op_array_len_typed(instruction),
            _ => unreachable!("exec_typed_array_elem_ops called with {:?}", instruction.opcode),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Resolve the absolute stack slot from the instruction's Local operand.
    #[inline(always)]
    fn resolve_local_slot(
        &self,
        instruction: &Instruction,
    ) -> Result<usize, VMError> {
        match instruction.operand {
            Some(Operand::Local(idx)) => {
                let slot = self.current_locals_base() + idx as usize;
                if slot >= self.stack.len() {
                    return Err(VMError::RuntimeError(format!(
                        "Local slot {} out of bounds (stack size {})",
                        idx,
                        self.stack.len()
                    )));
                }
                Ok(slot)
            }
            _ => Err(VMError::InvalidOperand),
        }
    }

    // -----------------------------------------------------------------------
    // GetElemI64
    // -----------------------------------------------------------------------

    fn op_get_elem_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let index = self.pop_raw_i64()?;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let result = self.stack_peek_raw(slot, |vw| -> Result<i64, VMError> {
            if let Some(hv) = vw.as_heap_ref() {
                match hv {
                    HeapValue::TypedArray(TypedArrayData::I64(buf)) => {
                        if index >= buf.data.len() {
                            return Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            });
                        }
                        Ok(buf.data[index])
                    }
                    HeapValue::Array(arr) => {
                        if index >= arr.len() {
                            return Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: arr.len(),
                            });
                        }
                        arr[index].as_i64().ok_or(VMError::TypeError {
                            expected: "int",
                            got: "non-int element",
                        })
                    }
                    _ => Err(VMError::TypeError {
                        expected: "Array<int>",
                        got: hv.type_name(),
                    }),
                }
            } else {
                Err(VMError::TypeError {
                    expected: "Array<int>",
                    got: "non-heap value",
                })
            }
        })?;

        self.push_raw_u64(ValueWord::from_i64(result))
    }

    // -----------------------------------------------------------------------
    // GetElemF64
    // -----------------------------------------------------------------------

    fn op_get_elem_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let index = self.pop_raw_i64()?;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let result = self.stack_peek_raw(slot, |vw| -> Result<f64, VMError> {
            if let Some(hv) = vw.as_heap_ref() {
                match hv {
                    HeapValue::TypedArray(TypedArrayData::F64(buf)) => {
                        if index >= buf.data.len() {
                            return Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            });
                        }
                        Ok(buf.data[index])
                    }
                    HeapValue::Array(arr) => {
                        if index >= arr.len() {
                            return Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: arr.len(),
                            });
                        }
                        arr[index].as_f64().ok_or(VMError::TypeError {
                            expected: "number",
                            got: "non-number element",
                        })
                    }
                    _ => Err(VMError::TypeError {
                        expected: "Array<number>",
                        got: hv.type_name(),
                    }),
                }
            } else {
                Err(VMError::TypeError {
                    expected: "Array<number>",
                    got: "non-heap value",
                })
            }
        })?;

        self.push_raw_u64(ValueWord::from_f64(result))
    }

    // -----------------------------------------------------------------------
    // SetElemI64
    // -----------------------------------------------------------------------

    fn op_set_elem_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let value_bits = self.pop_raw_u64()?;
        let index = self.pop_raw_i64()?;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let value = ValueWord::from_raw_bits(value_bits);
        let val = value.as_i64().ok_or(VMError::TypeError {
            expected: "int",
            got: "non-int value",
        })?;
        std::mem::forget(value);

        // Take-mutate-write pattern for the local slot.
        let mut arr_vw = self.stack_take_raw(slot);
        let result = if let Some(hv) = arr_vw.as_heap_mut() {
            match hv {
                HeapValue::TypedArray(TypedArrayData::I64(buf)) => {
                    let buf = Arc::make_mut(buf);
                    if index >= buf.data.len() {
                        Err(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: buf.data.len(),
                        })
                    } else {
                        buf.data[index] = val;
                        Ok(())
                    }
                }
                HeapValue::Array(arr) => {
                    let arr = Arc::make_mut(arr);
                    if index >= arr.len() {
                        Err(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: arr.len(),
                        })
                    } else {
                        arr[index] = ValueWord::from_i64(val);
                        Ok(())
                    }
                }
                _ => Err(VMError::TypeError {
                    expected: "Array<int>",
                    got: hv.type_name(),
                }),
            }
        } else {
            Err(VMError::TypeError {
                expected: "Array<int>",
                got: "non-heap value",
            })
        };
        self.stack_write_raw(slot, arr_vw);
        result
    }

    // -----------------------------------------------------------------------
    // SetElemF64
    // -----------------------------------------------------------------------

    fn op_set_elem_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let value_bits = self.pop_raw_u64()?;
        let index = self.pop_raw_i64()?;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let value = ValueWord::from_raw_bits(value_bits);
        let val = value.as_f64().ok_or(VMError::TypeError {
            expected: "number",
            got: "non-number value",
        })?;
        std::mem::forget(value);

        let mut arr_vw = self.stack_take_raw(slot);
        let result = if let Some(hv) = arr_vw.as_heap_mut() {
            match hv {
                HeapValue::TypedArray(TypedArrayData::F64(buf)) => {
                    let buf = Arc::make_mut(buf);
                    if index >= buf.data.len() {
                        Err(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: buf.data.len(),
                        })
                    } else {
                        buf.data[index] = val;
                        Ok(())
                    }
                }
                HeapValue::Array(arr) => {
                    let arr = Arc::make_mut(arr);
                    if index >= arr.len() {
                        Err(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: arr.len(),
                        })
                    } else {
                        arr[index] = ValueWord::from_f64(val);
                        Ok(())
                    }
                }
                _ => Err(VMError::TypeError {
                    expected: "Array<number>",
                    got: hv.type_name(),
                }),
            }
        } else {
            Err(VMError::TypeError {
                expected: "Array<number>",
                got: "non-heap value",
            })
        };
        self.stack_write_raw(slot, arr_vw);
        result
    }

    // -----------------------------------------------------------------------
    // ArrayPushI64
    // -----------------------------------------------------------------------

    fn op_array_push_i64_elem(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let value_bits = self.pop_raw_u64()?;

        let value = ValueWord::from_raw_bits(value_bits);
        let val = value.as_i64().ok_or(VMError::TypeError {
            expected: "int",
            got: "non-int value",
        })?;
        std::mem::forget(value);

        let mut arr_vw = self.stack_take_raw(slot);
        let result = if let Some(hv) = arr_vw.as_heap_mut() {
            match hv {
                HeapValue::TypedArray(TypedArrayData::I64(buf)) => {
                    Arc::make_mut(buf).data.push(val);
                    Ok(())
                }
                HeapValue::Array(arr) => {
                    Arc::make_mut(arr).push(ValueWord::from_i64(val));
                    Ok(())
                }
                _ => Err(VMError::TypeError {
                    expected: "Array<int>",
                    got: hv.type_name(),
                }),
            }
        } else {
            Err(VMError::TypeError {
                expected: "Array<int>",
                got: "non-heap value",
            })
        };
        self.stack_write_raw(slot, arr_vw);
        result
    }

    // -----------------------------------------------------------------------
    // ArrayPushF64
    // -----------------------------------------------------------------------

    fn op_array_push_f64_elem(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let value_bits = self.pop_raw_u64()?;

        let value = ValueWord::from_raw_bits(value_bits);
        let val = value.as_f64().ok_or(VMError::TypeError {
            expected: "number",
            got: "non-number value",
        })?;
        std::mem::forget(value);

        let mut arr_vw = self.stack_take_raw(slot);
        let result = if let Some(hv) = arr_vw.as_heap_mut() {
            match hv {
                HeapValue::TypedArray(TypedArrayData::F64(buf)) => {
                    Arc::make_mut(buf).data.push(val);
                    Ok(())
                }
                HeapValue::Array(arr) => {
                    Arc::make_mut(arr).push(ValueWord::from_f64(val));
                    Ok(())
                }
                _ => Err(VMError::TypeError {
                    expected: "Array<number>",
                    got: hv.type_name(),
                }),
            }
        } else {
            Err(VMError::TypeError {
                expected: "Array<number>",
                got: "non-heap value",
            })
        };
        self.stack_write_raw(slot, arr_vw);
        result
    }

    // -----------------------------------------------------------------------
    // ArrayLenTyped
    // -----------------------------------------------------------------------

    fn op_array_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;

        let len = self.stack_peek_raw(slot, |vw| -> Result<usize, VMError> {
            if let Some(hv) = vw.as_heap_ref() {
                match hv {
                    HeapValue::TypedArray(ta) => Ok(match ta {
                        TypedArrayData::I64(buf) => buf.data.len(),
                        TypedArrayData::F64(buf) => buf.data.len(),
                        TypedArrayData::Bool(buf) => buf.data.len(),
                        TypedArrayData::I8(buf) => buf.data.len(),
                        TypedArrayData::I16(buf) => buf.data.len(),
                        TypedArrayData::I32(buf) => buf.data.len(),
                        TypedArrayData::U8(buf) => buf.data.len(),
                        TypedArrayData::U16(buf) => buf.data.len(),
                        TypedArrayData::U32(buf) => buf.data.len(),
                        TypedArrayData::U64(buf) => buf.data.len(),
                        TypedArrayData::F32(buf) => buf.data.len(),
                        TypedArrayData::Matrix(m) => m.data.len(),
                        TypedArrayData::FloatSlice { len, .. } => *len as usize,
                    }),
                    HeapValue::Array(arr) => Ok(arr.len()),
                    _ => Err(VMError::TypeError {
                        expected: "array",
                        got: hv.type_name(),
                    }),
                }
            } else {
                Err(VMError::TypeError {
                    expected: "array",
                    got: "non-heap value",
                })
            }
        })?;

        self.push_raw_u64(ValueWord::from_i64(len as i64))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::bytecode::{Instruction, OpCode, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::heap_value::TypedArrayData;
    use shape_value::typed_buffer::TypedBuffer;
    use shape_value::{VMError, ValueWord, ValueWordExt};

    /// Create a minimal VM with enough stack space for testing.
    fn make_test_vm() -> VirtualMachine {
        let mut vm = VirtualMachine::new(VMConfig::default());
        // We need at least one stack slot for the array local.
        // Push a placeholder (will be overwritten by store).
        vm.push_raw_u64(ValueWord::none()).unwrap();
        vm.push_raw_u64(ValueWord::none()).unwrap();
        vm
    }

    /// Store a ValueWord into local slot 0 (absolute slot 0, since no call frames).
    fn store_local(vm: &mut VirtualMachine, slot: usize, value: ValueWord) {
        vm.stack_write_raw(slot, value);
    }

    /// Build an `IntArray` (TypedArrayData::I64) from a slice of i64 values.
    fn make_int_array(values: &[i64]) -> ValueWord {
        let buf = TypedBuffer::from_vec(values.to_vec());
        ValueWord::from_int_array(Arc::new(buf))
    }

    /// Build a `FloatArray` (TypedArrayData::F64) from a slice of f64 values.
    fn make_float_array(values: &[f64]) -> ValueWord {
        use shape_value::typed_buffer::AlignedTypedBuffer;
        use shape_value::aligned_vec::AlignedVec;
        let mut av = AlignedVec::with_capacity(values.len());
        for &v in values {
            av.push(v);
        }
        let buf = AlignedTypedBuffer::from_aligned(av);
        ValueWord::from_float_array(Arc::new(buf))
    }

    // ---- GetElemI64 on IntArray ----

    #[test]
    fn test_get_elem_i64_typed_array() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10, 20, 30]));

        // Push index 1 onto the stack, then execute GetElemI64.
        vm.push_raw_u64(ValueWord::from_i64(1)).unwrap();
        let instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        let result = vm.pop_raw_u64().unwrap();
        let vw = ValueWord::from_raw_bits(result);
        assert_eq!(vw.as_i64(), Some(20));
    }

    // ---- GetElemF64 on FloatArray ----

    #[test]
    fn test_get_elem_f64_typed_array() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_float_array(&[1.5, 2.7, 3.14]));

        vm.push_raw_u64(ValueWord::from_i64(2)).unwrap();
        let instr = Instruction::new(OpCode::GetElemF64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        let result = vm.pop_raw_u64().unwrap();
        let vw = ValueWord::from_raw_bits(result);
        assert!((vw.as_f64().unwrap() - 3.14).abs() < 1e-12);
    }

    // ---- SetElemI64 mutation ----

    #[test]
    fn test_set_elem_i64() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10, 20, 30]));

        // SetElemI64: pops [index, value] — push index first, then value.
        vm.push_raw_u64(ValueWord::from_i64(1)).unwrap(); // index
        vm.push_raw_u64(ValueWord::from_i64(999)).unwrap(); // value
        let instr = Instruction::new(OpCode::SetElemI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        // Verify by reading back.
        vm.push_raw_u64(ValueWord::from_i64(1)).unwrap();
        let get_instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&get_instr).unwrap();
        let result = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(result).as_i64(), Some(999));
    }

    // ---- SetElemF64 mutation ----

    #[test]
    fn test_set_elem_f64() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_float_array(&[1.0, 2.0, 3.0]));

        vm.push_raw_u64(ValueWord::from_i64(0)).unwrap(); // index
        vm.push_raw_u64(ValueWord::from_f64(42.5)).unwrap(); // value
        let instr = Instruction::new(OpCode::SetElemF64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        // Verify.
        vm.push_raw_u64(ValueWord::from_i64(0)).unwrap();
        let get_instr = Instruction::new(OpCode::GetElemF64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&get_instr).unwrap();
        let result = vm.pop_raw_u64().unwrap();
        assert!((ValueWord::from_raw_bits(result).as_f64().unwrap() - 42.5).abs() < 1e-12);
    }

    // ---- ArrayPushI64 append ----

    #[test]
    fn test_array_push_i64() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10, 20]));

        // Push value 30.
        vm.push_raw_u64(ValueWord::from_i64(30)).unwrap();
        let instr = Instruction::new(OpCode::ArrayPushI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        // Verify length is now 3.
        let len_instr = Instruction::new(OpCode::ArrayLenTyped, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&len_instr).unwrap();
        let len = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(len).as_i64(), Some(3));

        // Verify element at index 2.
        vm.push_raw_u64(ValueWord::from_i64(2)).unwrap();
        let get_instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&get_instr).unwrap();
        let result = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(result).as_i64(), Some(30));
    }

    // ---- ArrayPushF64 append ----

    #[test]
    fn test_array_push_f64() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_float_array(&[1.0]));

        vm.push_raw_u64(ValueWord::from_f64(2.5)).unwrap();
        let instr = Instruction::new(OpCode::ArrayPushF64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        // Verify length is 2.
        let len_instr = Instruction::new(OpCode::ArrayLenTyped, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&len_instr).unwrap();
        let len = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(len).as_i64(), Some(2));

        // Verify element at index 1.
        vm.push_raw_u64(ValueWord::from_i64(1)).unwrap();
        let get_instr = Instruction::new(OpCode::GetElemF64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&get_instr).unwrap();
        let result = vm.pop_raw_u64().unwrap();
        assert!((ValueWord::from_raw_bits(result).as_f64().unwrap() - 2.5).abs() < 1e-12);
    }

    // ---- ArrayLenTyped ----

    #[test]
    fn test_array_len_typed() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[1, 2, 3, 4, 5]));

        let instr = Instruction::new(OpCode::ArrayLenTyped, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();
        let len = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(len).as_i64(), Some(5));
    }

    #[test]
    fn test_array_len_typed_float() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_float_array(&[1.0, 2.0]));

        let instr = Instruction::new(OpCode::ArrayLenTyped, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();
        let len = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(len).as_i64(), Some(2));
    }

    // ---- Out-of-bounds error handling ----

    #[test]
    fn test_get_elem_i64_out_of_bounds() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10, 20]));

        vm.push_raw_u64(ValueWord::from_i64(5)).unwrap();
        let instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        let err = vm.exec_typed_array_elem_ops(&instr);
        assert!(err.is_err());
        match err.unwrap_err() {
            VMError::IndexOutOfBounds { index, length } => {
                assert_eq!(index, 5);
                assert_eq!(length, 2);
            }
            other => panic!("Expected IndexOutOfBounds, got {:?}", other),
        }
    }

    #[test]
    fn test_get_elem_i64_negative_index() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10, 20]));

        vm.push_raw_u64(ValueWord::from_i64(-1)).unwrap();
        let instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        let err = vm.exec_typed_array_elem_ops(&instr);
        assert!(err.is_err());
    }

    #[test]
    fn test_set_elem_i64_out_of_bounds() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_int_array(&[10]));

        vm.push_raw_u64(ValueWord::from_i64(5)).unwrap(); // index
        vm.push_raw_u64(ValueWord::from_i64(999)).unwrap(); // value
        let instr = Instruction::new(OpCode::SetElemI64, Some(Operand::Local(0)));
        let err = vm.exec_typed_array_elem_ops(&instr);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_elem_f64_out_of_bounds() {
        let mut vm = make_test_vm();
        store_local(&mut vm, 0, make_float_array(&[1.0]));

        vm.push_raw_u64(ValueWord::from_i64(10)).unwrap();
        let instr = Instruction::new(OpCode::GetElemF64, Some(Operand::Local(0)));
        let err = vm.exec_typed_array_elem_ops(&instr);
        assert!(err.is_err());
    }

    // ---- Generic Array fallback ----

    #[test]
    fn test_get_elem_i64_generic_array_fallback() {
        let mut vm = make_test_vm();
        // Create a generic Array (not TypedArrayData).
        let arr: Arc<Vec<ValueWord>> = Arc::new(vec![
            ValueWord::from_i64(100),
            ValueWord::from_i64(200),
        ]);
        let vw = ValueWord::from_heap_value(shape_value::HeapValue::Array(arr));
        store_local(&mut vm, 0, vw);

        vm.push_raw_u64(ValueWord::from_i64(1)).unwrap();
        let instr = Instruction::new(OpCode::GetElemI64, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();

        let result = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(result).as_i64(), Some(200));
    }

    #[test]
    fn test_array_len_typed_generic_array() {
        let mut vm = make_test_vm();
        let arr: Arc<Vec<ValueWord>> = Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]);
        let vw = ValueWord::from_heap_value(shape_value::HeapValue::Array(arr));
        store_local(&mut vm, 0, vw);

        let instr = Instruction::new(OpCode::ArrayLenTyped, Some(Operand::Local(0)));
        vm.exec_typed_array_elem_ops(&instr).unwrap();
        let len = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(len).as_i64(), Some(3));
    }
}
