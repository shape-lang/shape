//! Typed HashMap and String access opcodes — local-slot based, skip HeapValue dispatch.
//!
//! These handlers operate on HashMap / String values stored in local variable
//! slots, accessed via `Operand::Local(slot)`. The key/index comes from the
//! stack. This avoids the full `GetProp` / `CallMethod` dispatch overhead for
//! statically-typed access patterns the compiler can prove.

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::objects::raw_helpers;
use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    // =====================================================================
    // Typed HashMap access (local-slot based)
    // =====================================================================

    /// Dispatch for typed HashMap access opcodes (MapGetStrI64, MapGetStrF64,
    /// MapSetStrI64, MapHasStr, MapLenTyped).
    pub(in crate::executor) fn exec_typed_map_access(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::MapGetStrI64 => self.op_map_get_str_i64(instruction),
            OpCode::MapGetStrF64 => self.op_map_get_str_f64(instruction),
            OpCode::MapSetStrI64 => self.op_map_set_str_i64(instruction),
            OpCode::MapHasStr => self.op_map_has_str(instruction),
            OpCode::MapLenTyped => self.op_map_len_typed(instruction),
            _ => unreachable!(
                "exec_typed_map_access called with non-map opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// Helper: read the local slot index from the instruction operand.
    #[inline(always)]
    fn extract_local_slot(instruction: &Instruction) -> Result<u16, VMError> {
        match instruction.operand {
            Some(Operand::Local(idx)) => Ok(idx),
            _ => Err(VMError::InvalidOperand),
        }
    }

    /// MapGetStrI64: get value from HashMap<string, int>. Key on stack, map in local slot.
    /// Pushes the value (int) or none if key not found.
    fn op_map_get_str_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let key_bits = self.pop_raw_u64()?;

        let bp = self.current_locals_base();
        let map_bits = self.stack[bp + slot_idx as usize];

        if let Some(map_data) = raw_helpers::extract_hashmap_data(map_bits) {
            // Fast path for string keys: use shape_get
            if let Some(key_str) = raw_helpers::extract_str(key_bits) {
                if let Some(val) = map_data.shape_get(key_str) {
                    self.push_raw_u64(val.clone())?;
                    return Ok(());
                }
            }
            // Fallback: hash-based lookup
            let hash = key_bits.vw_hash();
            if let Some(bucket) = map_data.index.get(&hash) {
                for &idx in bucket {
                    if let (Some(k), Some(needle)) =
                        (raw_helpers::extract_str(map_data.keys[idx]), raw_helpers::extract_str(key_bits))
                    {
                        if k == needle {
                            self.push_raw_u64(map_data.values[idx].clone())?;
                            return Ok(());
                        }
                    }
                }
            }
            // Key not found
            self.push_raw_u64(Self::NONE_BITS)?;
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "HashMap",
                got: raw_helpers::type_name_from_bits(map_bits),
            })
        }
    }

    /// MapGetStrF64: get value from HashMap<string, float>. Key on stack, map in local slot.
    /// Pushes the value (float) or none if key not found.
    fn op_map_get_str_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let key_bits = self.pop_raw_u64()?;

        let bp = self.current_locals_base();
        let map_bits = self.stack[bp + slot_idx as usize];

        if let Some(map_data) = raw_helpers::extract_hashmap_data(map_bits) {
            // Fast path for string keys: use shape_get
            if let Some(key_str) = raw_helpers::extract_str(key_bits) {
                if let Some(val) = map_data.shape_get(key_str) {
                    self.push_raw_u64(val.clone())?;
                    return Ok(());
                }
            }
            // Fallback: hash-based lookup
            let hash = key_bits.vw_hash();
            if let Some(bucket) = map_data.index.get(&hash) {
                for &idx in bucket {
                    if let (Some(k), Some(needle)) =
                        (raw_helpers::extract_str(map_data.keys[idx]), raw_helpers::extract_str(key_bits))
                    {
                        if k == needle {
                            self.push_raw_u64(map_data.values[idx].clone())?;
                            return Ok(());
                        }
                    }
                }
            }
            // Key not found
            self.push_raw_u64(Self::NONE_BITS)?;
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "HashMap",
                got: raw_helpers::type_name_from_bits(map_bits),
            })
        }
    }

    /// MapSetStrI64: set value in HashMap<string, int>. Key and value on stack, map in local slot.
    /// Mutates the map in-place (or clones on write).
    fn op_map_set_str_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let value_bits = self.pop_raw_u64()?;
        let key_bits = self.pop_raw_u64()?;

        let bp = self.current_locals_base();
        let slot = bp + slot_idx as usize;

        // Clone the map value from the slot (bumps refcount), mutate, write back
        let mut map_vw = self.stack_read_raw(slot);
        if let Some(map_data) = map_vw.as_hashmap_mut() {
            let key = unsafe { ValueWord::clone_from_bits(key_bits) };
            let value = unsafe { ValueWord::clone_from_bits(value_bits) };
            let hash = key.vw_hash();

            // Check if key already exists
            if let Some(bucket) = map_data.index.get(&hash) {
                for &idx in bucket {
                    if map_data.keys[idx].vw_equals(&key) {
                        map_data.keys[idx] = key;
                        map_data.values[idx] = value;
                        self.stack_write_raw(slot, map_vw);
                        return Ok(());
                    }
                }
            }
            // Insert new key
            let new_idx = map_data.keys.len();
            // Transition shape if string key
            if let Some(shape_id) = map_data.shape_id {
                if let Some(ks) = key.as_str() {
                    let prop_hash = shape_value::hash_property_name(ks);
                    map_data.shape_id = shape_value::shape_transition(shape_id, prop_hash);
                } else {
                    map_data.shape_id = None;
                }
            }
            map_data.keys.push(key);
            map_data.values.push(value);
            map_data.index.entry(hash).or_default().push(new_idx);
            self.stack_write_raw(slot, map_vw);
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "HashMap",
                got: map_vw.type_name(),
            })
        }
    }

    /// MapHasStr: check if key exists in HashMap. Key on stack, map in local slot.
    /// Pushes bool.
    fn op_map_has_str(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let key_bits = self.pop_raw_u64()?;

        let bp = self.current_locals_base();
        let map_bits = self.stack[bp + slot_idx as usize];

        if let Some(map_data) = raw_helpers::extract_hashmap_data(map_bits) {
            // Fast path for string keys
            if let Some(key_str) = raw_helpers::extract_str(key_bits) {
                let found = map_data.shape_get(key_str).is_some();
                if found {
                    self.push_raw_u64(ValueWord::from_bool(true))?;
                    return Ok(());
                }
            }
            // Fallback: hash-based lookup
            let hash = key_bits.vw_hash();
            let found = if let Some(bucket) = map_data.index.get(&hash) {
                bucket.iter().any(|&idx| {
                    if let (Some(k), Some(needle)) =
                        (raw_helpers::extract_str(map_data.keys[idx]), raw_helpers::extract_str(key_bits))
                    {
                        k == needle
                    } else {
                        false
                    }
                })
            } else {
                false
            };
            self.push_raw_u64(ValueWord::from_bool(found))?;
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "HashMap",
                got: raw_helpers::type_name_from_bits(map_bits),
            })
        }
    }

    /// MapLenTyped: get HashMap length. Map in local slot. Pushes int.
    fn op_map_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let bp = self.current_locals_base();
        let map_bits = self.stack[bp + slot_idx as usize];

        if let Some(map_data) = raw_helpers::extract_hashmap_data(map_bits) {
            let len = map_data.keys.len();
            self.push_raw_u64(ValueWord::from_i64(len as i64))?;
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "HashMap",
                got: raw_helpers::type_name_from_bits(map_bits),
            })
        }
    }

    // =====================================================================
    // Typed String access (local-slot based or stack-based)
    // =====================================================================

    /// Dispatch for typed String access opcodes (StringLenTyped, StringCharAt,
    /// StringConcatTyped).
    pub(in crate::executor) fn exec_typed_string_access(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::StringLenTyped => self.op_string_len_typed(instruction),
            OpCode::StringCharAt => self.op_string_char_at(instruction),
            OpCode::StringConcatTyped => self.op_string_concat_typed(),
            _ => unreachable!(
                "exec_typed_string_access called with non-string opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// StringLenTyped: get string length (char count). String in local slot. Pushes int.
    fn op_string_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let bp = self.current_locals_base();
        let str_bits = self.stack[bp + slot_idx as usize];

        if let Some(s) = raw_helpers::extract_str(str_bits) {
            self.push_raw_u64(ValueWord::from_i64(s.chars().count() as i64))?;
            Ok(())
        } else {
            Err(VMError::TypeError {
                expected: "string",
                got: raw_helpers::type_name_from_bits(str_bits),
            })
        }
    }

    /// StringCharAt: get char at index. Index on stack, string in local slot. Pushes char.
    fn op_string_char_at(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let index_bits = self.pop_raw_u64()?;

        // Extract index as i64
        let index = raw_helpers::extract_i48(index_bits) as usize;

        let bp = self.current_locals_base();
        let str_bits = self.stack[bp + slot_idx as usize];

        if let Some(s) = raw_helpers::extract_str(str_bits) {
            if let Some(ch) = s.chars().nth(index) {
                self.push_raw_u64(ValueWord::from_char(ch))?;
                Ok(())
            } else {
                Err(VMError::IndexOutOfBounds {
                    index: index as i32,
                    length: s.chars().count(),
                })
            }
        } else {
            Err(VMError::TypeError {
                expected: "string",
                got: raw_helpers::type_name_from_bits(str_bits),
            })
        }
    }

    /// StringConcatTyped: concatenate two strings from the stack. Pushes result string.
    fn op_string_concat_typed(&mut self) -> Result<(), VMError> {
        let b_bits = self.pop_raw_u64()?;
        let a_bits = self.pop_raw_u64()?;

        let a = raw_helpers::extract_str(a_bits).ok_or(VMError::TypeError {
            expected: "string",
            got: raw_helpers::type_name_from_bits(a_bits),
        })?;
        let b = raw_helpers::extract_str(b_bits).ok_or(VMError::TypeError {
            expected: "string",
            got: raw_helpers::type_name_from_bits(b_bits),
        })?;

        let result = format!("{}{}", a, b);
        self.push_raw_u64(ValueWord::from_string(Arc::new(result)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{
        BytecodeProgram, Constant, Instruction, OpCode, Operand,
    };
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{ValueWord, ValueWordExt};
    use shape_value::heap_value::{HashMapData, HeapValue};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Helper: build a program, load it, execute, return the top-of-stack value.
    fn run_program(program: BytecodeProgram) -> ValueWord {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    /// Create a HashMap ValueWord with given string->int entries.
    fn make_str_int_map(entries: &[(&str, i64)]) -> ValueWord {
        let mut map_data = HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        };
        for (k, v) in entries {
            let key = ValueWord::from_string(Arc::new(k.to_string()));
            let hash = key.vw_hash();
            let idx = map_data.keys.len();
            map_data.keys.push(key);
            map_data.values.push(ValueWord::from_i64(*v));
            map_data.index.entry(hash).or_default().push(idx);
        }
        ValueWord::from_heap_value(HeapValue::HashMap(Box::new(map_data)))
    }

    /// Create a HashMap ValueWord with given string->f64 entries.
    fn make_str_f64_map(entries: &[(&str, f64)]) -> ValueWord {
        let mut map_data = HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        };
        for (k, v) in entries {
            let key = ValueWord::from_string(Arc::new(k.to_string()));
            let hash = key.vw_hash();
            let idx = map_data.keys.len();
            map_data.keys.push(key);
            map_data.values.push(ValueWord::from_f64(*v));
            map_data.index.entry(hash).or_default().push(idx);
        }
        ValueWord::from_heap_value(HeapValue::HashMap(Box::new(map_data)))
    }

    // ===== MapGetStrI64 =====

    #[test]
    fn test_map_get_str_i64_found() {
        let map_vw = make_str_int_map(&[("x", 42), ("y", 99)]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("x".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::MapGetStrI64, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_map_get_str_i64_not_found() {
        let map_vw = make_str_int_map(&[("x", 42)]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("missing".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::MapGetStrI64, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert!(result.is_none());
    }

    // ===== MapGetStrF64 =====

    #[test]
    fn test_map_get_str_f64_found() {
        let map_vw = make_str_f64_map(&[("pi", 3.14)]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("pi".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::MapGetStrF64, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(3.14));
    }

    // ===== MapHasStr =====

    #[test]
    fn test_map_has_str_true() {
        let map_vw = make_str_int_map(&[("key", 1)]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("key".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::MapHasStr, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_map_has_str_false() {
        let map_vw = make_str_int_map(&[]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("nope".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::MapHasStr, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_bool(), Some(false));
    }

    // ===== MapLenTyped =====

    #[test]
    fn test_map_len_typed() {
        let map_vw = make_str_int_map(&[("a", 1), ("b", 2), ("c", 3)]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::MapLenTyped, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(3));
    }

    #[test]
    fn test_map_len_empty() {
        let map_vw = make_str_int_map(&[]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::MapLenTyped, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(0));
    }

    // ===== MapSetStrI64 =====

    #[test]
    fn test_map_set_str_i64() {
        let map_vw = make_str_int_map(&[]);
        let mut program = BytecodeProgram::default();
        let c_map = program.add_constant(Constant::Value(map_vw));
        let c_key = program.add_constant(Constant::String("test".to_string()));
        let c_val = program.add_constant(Constant::Int(777));
        let c_key2 = program.add_constant(Constant::String("test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_map))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_val))),
            Instruction::new(OpCode::MapSetStrI64, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_key2))),
            Instruction::new(OpCode::MapGetStrI64, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(777));
    }

    // ===== StringLenTyped =====

    #[test]
    fn test_string_len_typed() {
        let mut program = BytecodeProgram::default();
        let c_str = program.add_constant(Constant::String("hello".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_str))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::StringLenTyped, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(5));
    }

    #[test]
    fn test_string_len_typed_empty() {
        let mut program = BytecodeProgram::default();
        let c_str = program.add_constant(Constant::String("".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_str))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::StringLenTyped, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(0));
    }

    // ===== StringCharAt =====

    #[test]
    fn test_string_char_at() {
        let mut program = BytecodeProgram::default();
        let c_str = program.add_constant(Constant::String("hello".to_string()));
        let c_idx = program.add_constant(Constant::Int(1));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_str))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_idx))),
            Instruction::new(OpCode::StringCharAt, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_char(), Some('e'));
    }

    #[test]
    fn test_string_char_at_out_of_bounds() {
        let mut program = BytecodeProgram::default();
        let c_str = program.add_constant(Constant::String("hi".to_string()));
        let c_idx = program.add_constant(Constant::Int(5));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_str))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_idx))),
            Instruction::new(OpCode::StringCharAt, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None);
        assert!(result.is_err());
    }

    // ===== StringConcatTyped =====

    #[test]
    fn test_string_concat_typed() {
        let mut program = BytecodeProgram::default();
        let c_a = program.add_constant(Constant::String("hello".to_string()));
        let c_b = program.add_constant(Constant::String(" world".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_a))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_b))),
            Instruction::simple(OpCode::StringConcatTyped),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 0;
        let result = run_program(program);
        assert_eq!(result.as_str(), Some("hello world"));
    }

    #[test]
    fn test_string_concat_typed_empty() {
        let mut program = BytecodeProgram::default();
        let c_a = program.add_constant(Constant::String("abc".to_string()));
        let c_b = program.add_constant(Constant::String("".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_a))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_b))),
            Instruction::simple(OpCode::StringConcatTyped),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 0;
        let result = run_program(program);
        assert_eq!(result.as_str(), Some("abc"));
    }
}
