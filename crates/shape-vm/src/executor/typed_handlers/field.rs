//! Typed struct field load/store handlers.
//!
//! These handlers execute FieldLoadXxx/FieldStoreXxx opcodes by reading/writing
//! at compile-time-known byte offsets into typed structs. The byte offset is
//! baked into the operand — no schema lookup at runtime.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::VirtualMachine;
use shape_value::VMError;

impl VirtualMachine {
    /// Execute a typed field opcode (FieldLoad/FieldStore/NewTypedStruct).
    pub(crate) fn exec_typed_field(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            // Field load handlers: the struct pointer is stored as raw bits
            // (a typed-struct allocation, NOT a heap-boxed ValueWord), so
            // we use raw pop/push to skip ValueWord materialization.
            OpCode::FieldLoadF64 => {
                let offset = instruction.operand_field_offset() as usize;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *const u8;
                // Safety: compiler has proven the type and offset at compile time.
                let val: f64 = unsafe { *(struct_ptr.add(offset) as *const f64) };
                self.push_raw_f64(val)
            }
            OpCode::FieldLoadI64 => {
                let offset = instruction.operand_field_offset() as usize;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *const u8;
                let val: i64 = unsafe { *(struct_ptr.add(offset) as *const i64) };
                self.push_raw_i64(val)
            }
            OpCode::FieldLoadI32 => {
                let offset = instruction.operand_field_offset() as usize;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *const u8;
                let val: i32 = unsafe { *(struct_ptr.add(offset) as *const i32) };
                // Widen to i64 for the NaN-boxed stack
                self.push_raw_i64(val as i64)
            }
            OpCode::FieldLoadBool => {
                let offset = instruction.operand_field_offset() as usize;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *const u8;
                let val: u8 = unsafe { *struct_ptr.add(offset) };
                self.push_raw_bool(val != 0)
            }
            OpCode::FieldLoadPtr => {
                let offset = instruction.operand_field_offset() as usize;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *const u8;
                // Load a raw pointer-sized value and push as raw bits.
                // Downstream consumers handle the bit pattern (NaN-boxed
                // ValueWord or raw typed pointer, depending on field type).
                let val: u64 = unsafe { *(struct_ptr.add(offset) as *const u64) };
                self.push_raw_u64(val)
            }
            OpCode::FieldStoreF64 => {
                let offset = instruction.operand_field_offset() as usize;
                let f = self.pop_raw_f64()?;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut f64) = f };
                Ok(())
            }
            OpCode::FieldStoreI64 => {
                let offset = instruction.operand_field_offset() as usize;
                let i = self.pop_raw_i64()?;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut i64) = i };
                Ok(())
            }
            OpCode::FieldStoreI32 => {
                let offset = instruction.operand_field_offset() as usize;
                let i = self.pop_raw_i64()? as i32;
                let struct_bits = self.pop_raw_u64()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut i32) = i };
                Ok(())
            }
            OpCode::NewTypedStruct => {
                // Operand: TypedObjectAlloc { schema_id, field_count }
                // field_count encodes the total struct size in bytes.
                let (schema_id, total_size) = match instruction.operand {
                    Some(crate::bytecode::Operand::TypedObjectAlloc {
                        schema_id,
                        field_count,
                    }) => (schema_id, field_count as usize),
                    _ => {
                        return Err(VMError::RuntimeError(
                            "NewTypedStruct requires TypedObjectAlloc operand".into(),
                        ));
                    }
                };
                // Allocate zeroed memory for the struct (includes HeapHeader space)
                let layout = std::alloc::Layout::from_size_align(total_size, 8)
                    .map_err(|_| VMError::RuntimeError("invalid struct layout".into()))?;
                let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                if ptr.is_null() {
                    return Err(VMError::RuntimeError("struct allocation failed".into()));
                }
                // Initialize HeapHeader: refcount=1, kind=schema_id
                unsafe {
                    // refcount at offset 0 (AtomicU32 stored as u32)
                    *(ptr as *mut u32) = 1;
                    // kind at offset 4 (u16)
                    *(ptr.add(4) as *mut u16) = schema_id;
                }
                // Store the raw pointer directly as a u64 — this is a typed
                // allocation, NOT a standard Arc<HeapValue>, so we bypass
                // ValueWord materialization. The typed drop/refcount path must
                // handle deallocation.
                self.push_raw_u64(ptr as u64)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled typed field opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
