//! v2 typed enum handlers — tag dispatch and payload field load.
//!
//! These handlers execute the `EnumTagLoad` and `EnumPayloadField` opcodes
//! emitted by the typed match dispatch path (Phase 3.3). The compiler has
//! proven the value on the stack is a typed enum heap pointer, so the
//! handlers do no runtime type checking — they just read the tag byte at
//! offset 8 (past the HeapHeader) and payload fields at compile-time-known
//! offsets within the payload area starting at offset 16.
//!
//! See `shape_value::v2::enum_layout` for the heap layout.
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C: kinded API. Receiver kind is
//! `NativeKind::Ptr(HeapKind::TypedObject)` (typed enum payload).
//! `EnumTagLoad` pushes `NativeKind::Int64`; `EnumPayloadField` pushes
//! `NativeKind::Int64` for the raw 8-byte payload slot.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapKind;
use shape_value::v2::enum_layout::{ENUM_PAYLOAD_OFFSET, ENUM_TAG_OFFSET};
use shape_value::{NativeKind, VMError};

impl VirtualMachine {
    /// Execute a v2 typed enum opcode (`EnumTagLoad` / `EnumPayloadField`).
    pub(crate) fn exec_v2_typed_enum(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::EnumTagLoad => {
                // Pop the typed enum heap pointer. Receiver kind is the
                // typed-object Ptr arm.
                let (enum_bits, enum_kind) = self.pop_kinded()?;
                let enum_ptr = enum_bits as *const u8;
                if enum_ptr.is_null() {
                    drop_with_kind(enum_bits, enum_kind);
                    return Err(VMError::RuntimeError(
                        "EnumTagLoad: null typed enum pointer".into(),
                    ));
                }
                // Safety: the compiler proved this is a v2 typed enum allocation
                // (header + tag + payload). Reading the tag byte at offset 8 is
                // always in-bounds for any valid typed enum object.
                let tag: u8 = unsafe { *enum_ptr.add(ENUM_TAG_OFFSET) };
                drop_with_kind(enum_bits, enum_kind);
                // Push as i64 (sign-extended through u8 → i64).
                self.push_kinded(tag as i64 as u64, NativeKind::Int64)
            }
            OpCode::EnumPayloadField => {
                // Operand encodes the byte offset within the payload area.
                let payload_offset = instruction.operand_field_offset() as usize;
                let (enum_bits, enum_kind) = self.pop_kinded()?;
                let enum_ptr = enum_bits as *const u8;
                if enum_ptr.is_null() {
                    drop_with_kind(enum_bits, enum_kind);
                    return Err(VMError::RuntimeError(
                        "EnumPayloadField: null typed enum pointer".into(),
                    ));
                }
                // Safety: the compiler emitted this opcode immediately after a
                // tag check that established which variant is live, and the
                // payload offset was looked up from that variant's layout.
                let field_ptr = unsafe { enum_ptr.add(ENUM_PAYLOAD_OFFSET + payload_offset) };
                // Read 8 bytes — the widest payload slot. Downstream typed
                // opcodes will reinterpret as needed.
                let raw: u64 = unsafe { std::ptr::read_unaligned(field_ptr as *const u64) };
                drop_with_kind(enum_bits, enum_kind);
                let _ = HeapKind::TypedObject; // marker for the receiver-side arm
                self.push_kinded(raw, NativeKind::Int64)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled v2 enum opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::enum_layout::compute_enum_layout;
    use shape_value::v2::struct_layout::FieldKind;
    use shape_value::v2::typed_enum::{
        alloc_typed_enum, dealloc_typed_enum, write_payload_field, write_tag,
    };

    /// Helper: build a tiny `Color { Red, Green, Blue }` layout and a heap
    /// object with the requested tag, returning the raw pointer (caller frees).
    fn make_color_with_tag(tag: u8) -> (*mut u8, shape_value::v2::EnumLayout) {
        let variants = vec![
            ("Red".to_string(), vec![]),
            ("Green".to_string(), vec![]),
            ("Blue".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("Color", &variants);
        let ptr = alloc_typed_enum(&layout);
        unsafe { write_tag(ptr, tag) };
        (ptr, layout)
    }

    #[test]
    fn enum_tag_load_reads_tag_byte() {
        // Build a Color::Green object (tag = 1) and read its tag through the
        // raw u8 path used by the EnumTagLoad opcode.
        let (ptr, layout) = make_color_with_tag(1);
        let tag = unsafe { *(ptr as *const u8).add(ENUM_TAG_OFFSET) };
        assert_eq!(tag, 1);
        unsafe { dealloc_typed_enum(&layout, ptr) };
    }

    #[test]
    fn enum_payload_field_reads_i64() {
        // enum E { Pair(i64, i64) }
        let variants = vec![(
            "Pair".to_string(),
            vec![FieldKind::I64, FieldKind::I64],
        )];
        let layout = compute_enum_layout("E", &variants);
        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 0);
            write_payload_field(ptr, 0, FieldKind::I64, 42_i64 as u64);
            write_payload_field(ptr, 8, FieldKind::I64, 99_i64 as u64);
        }

        // Simulate the executor's read at the second field offset.
        let payload_offset_1 = 8usize;
        let raw = unsafe {
            let field_ptr = (ptr as *const u8).add(ENUM_PAYLOAD_OFFSET + payload_offset_1);
            std::ptr::read_unaligned(field_ptr as *const u64)
        };
        assert_eq!(raw as i64, 99);

        unsafe { dealloc_typed_enum(&layout, ptr) };
    }
}
