//! Typed struct field load/store handlers.
//!
//! These handlers execute FieldLoadXxx/FieldStoreXxx opcodes by reading/writing
//! at compile-time-known byte offsets into typed structs. The byte offset is
//! baked into the operand — no schema lookup at runtime.
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C: kinded API. Receiver kind is the
//! raw typed-struct pointer (allocated by `NewTypedStruct`), pushed as
//! `NativeKind::UInt64` (NativeScalar shape — non-Arc, no refcount).
//! Loaded scalar fields use the per-FieldKind native kind (Float64, Int64,
//! Int32, Bool); pointer fields keep raw bits with `NativeKind::UInt64`.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::{NativeKind, VMError};

impl VirtualMachine {
    /// Execute a typed field opcode (FieldLoad/FieldStore/NewTypedStruct).
    pub(crate) fn exec_typed_field(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            // Field load handlers: the struct pointer is stored as raw bits
            // (a typed-struct allocation, NOT an Arc<HeapValue>), so the
            // pop/push goes through the kinded API without refcount churn —
            // UInt64 selects the no-op Drop arm.
            OpCode::FieldLoadF64 => {
                let offset = instruction.operand_field_offset() as usize;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *const u8;
                // Safety: compiler has proven the type and offset at compile time.
                let val: f64 = unsafe { *(struct_ptr.add(offset) as *const f64) };
                drop_with_kind(struct_bits, struct_kind);
                self.push_kinded(val.to_bits(), NativeKind::Float64)
            }
            OpCode::FieldLoadI64 => {
                let offset = instruction.operand_field_offset() as usize;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *const u8;
                let val: i64 = unsafe { *(struct_ptr.add(offset) as *const i64) };
                drop_with_kind(struct_bits, struct_kind);
                self.push_kinded(val as u64, NativeKind::Int64)
            }
            OpCode::FieldLoadI32 => {
                let offset = instruction.operand_field_offset() as usize;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *const u8;
                let val: i32 = unsafe { *(struct_ptr.add(offset) as *const i32) };
                drop_with_kind(struct_bits, struct_kind);
                // Widen to i64 (sign-extended) — typed kind track distinguishes
                // Int32 vs Int64 so the consumer can downcast cleanly.
                self.push_kinded(val as i64 as u64, NativeKind::Int32)
            }
            OpCode::FieldLoadBool => {
                let offset = instruction.operand_field_offset() as usize;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *const u8;
                let val: u8 = unsafe { *struct_ptr.add(offset) };
                drop_with_kind(struct_bits, struct_kind);
                self.push_kinded((val != 0) as u64, NativeKind::Bool)
            }
            OpCode::FieldLoadPtr => {
                let offset = instruction.operand_field_offset() as usize;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *const u8;
                // Load a raw pointer-sized value and push as raw bits.
                // Downstream consumers handle the bit pattern (raw typed
                // pointer or NativeScalar — kind selects the no-op arm).
                let val: u64 = unsafe { *(struct_ptr.add(offset) as *const u64) };
                drop_with_kind(struct_bits, struct_kind);
                self.push_kinded(val, NativeKind::UInt64)
            }
            OpCode::FieldStoreF64 => {
                let offset = instruction.operand_field_offset() as usize;
                let (f_bits, _f_kind) = self.pop_kinded()?;
                let f = f64::from_bits(f_bits);
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut f64) = f };
                drop_with_kind(struct_bits, struct_kind);
                Ok(())
            }
            OpCode::FieldStoreI64 => {
                let offset = instruction.operand_field_offset() as usize;
                let (i_bits, _i_kind) = self.pop_kinded()?;
                let i = i_bits as i64;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut i64) = i };
                drop_with_kind(struct_bits, struct_kind);
                Ok(())
            }
            OpCode::FieldStoreI32 => {
                let offset = instruction.operand_field_offset() as usize;
                let (i_bits, _i_kind) = self.pop_kinded()?;
                let i = i_bits as i64 as i32;
                let (struct_bits, struct_kind) = self.pop_kinded()?;
                let struct_ptr = struct_bits as *mut u8;
                unsafe { *(struct_ptr.add(offset) as *mut i32) = i };
                drop_with_kind(struct_bits, struct_kind);
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
                // Raw typed-struct pointer: NativeScalar shape, not Arc-backed.
                // UInt64 kind selects the no-op Drop arm — the typed drop path
                // (FieldStorePtr / object Drop op) handles deallocation.
                self.push_kinded(ptr as u64, NativeKind::UInt64)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled typed field opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
