//! Dedicated concatenation opcodes (StringConcat, ArrayConcat).
//!
//! These replace the generic `OpCode::Add` overload for built-in heap types
//! whose operand types the compiler can prove statically. Operator overloading
//! on user-defined types still goes through `CallMethod` (see Phase 2.5).

use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

impl VirtualMachine {
    /// Concatenate two heap strings/chars, push the resulting string.
    ///
    /// Stack: `[a, b]` → `[a ++ b]`. Accepts any combination of
    /// `String + String`, `String + Char`, `Char + String`, `Char + Char`.
    /// All other operand combinations are a runtime type error (the compiler
    /// is supposed to only emit this opcode when both operands are statically
    /// proven to be `string` or `char`).
    #[inline]
    pub(in crate::executor) fn op_string_concat(&mut self) -> Result<(), VMError> {
        let b_bits = self.pop_raw_u64()?;
        let a_bits = self.pop_raw_u64()?;
        let a = ValueWord::from_raw_bits(a_bits);
        let b = ValueWord::from_raw_bits(b_bits);

        let result = match (a.as_heap_ref(), b.as_heap_ref()) {
            (Some(HeapValue::String(s_a)), Some(HeapValue::String(s_b))) => {
                format!("{}{}", s_a, s_b)
            }
            (Some(HeapValue::String(s)), Some(HeapValue::Char(c))) => format!("{}{}", s, c),
            (Some(HeapValue::Char(c)), Some(HeapValue::String(s))) => format!("{}{}", c, s),
            (Some(HeapValue::Char(c_a)), Some(HeapValue::Char(c_b))) => format!("{}{}", c_a, c_b),
            _ => {
                return Err(VMError::TypeError {
                    expected: "string or char operands for StringConcat",
                    got: a.type_name(),
                });
            }
        };
        self.push_vw(ValueWord::from_string(Arc::new(result)))
    }
}
