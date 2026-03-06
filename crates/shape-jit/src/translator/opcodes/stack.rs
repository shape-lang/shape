//! Stack operations: PushConst, PushNull, Pop, Dup, Swap

use cranelift::prelude::*;

use crate::context::*;
use crate::nan_boxing::*;
use shape_vm::bytecode::{Constant, Instruction, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::storage::TypedValue;
use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    pub(crate) fn compile_push_const(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Const(idx)) = &instr.operand {
            match &self.program.constants[*idx as usize] {
                Constant::Number(n) => {
                    use shape_vm::bytecode::OpCode;
                    // Find the first nearby numeric consumer opcode. If it's a typed
                    // float op/cmp, keep Number constants boxed to preserve f64 semantics.
                    let mut consumer_is_typed_float = false;
                    let end = (self.current_instr_idx + 8).min(self.program.instructions.len());
                    for j in (self.current_instr_idx + 1)..end {
                        let op = self.program.instructions[j].opcode;
                        match op {
                            OpCode::AddNumber
                            | OpCode::SubNumber
                            | OpCode::MulNumber
                            | OpCode::DivNumber
                            | OpCode::ModNumber
                            | OpCode::PowNumber
                            | OpCode::GtNumber
                            | OpCode::LtNumber
                            | OpCode::GteNumber
                            | OpCode::LteNumber
                            | OpCode::EqNumber
                            | OpCode::NeqNumber
                            | OpCode::AddNumberTrusted
                            | OpCode::SubNumberTrusted
                            | OpCode::MulNumberTrusted
                            | OpCode::DivNumberTrusted
                            | OpCode::GtNumberTrusted
                            | OpCode::LtNumberTrusted
                            | OpCode::GteNumberTrusted
                            | OpCode::LteNumberTrusted => {
                                consumer_is_typed_float = true;
                                break;
                            }
                            OpCode::AddInt
                            | OpCode::SubInt
                            | OpCode::MulInt
                            | OpCode::DivInt
                            | OpCode::ModInt
                            | OpCode::PowInt
                            | OpCode::GtInt
                            | OpCode::LtInt
                            | OpCode::GteInt
                            | OpCode::LteInt
                            | OpCode::EqInt
                            | OpCode::NeqInt
                            | OpCode::AddIntTrusted
                            | OpCode::SubIntTrusted
                            | OpCode::MulIntTrusted
                            | OpCode::DivIntTrusted
                            | OpCode::GtIntTrusted
                            | OpCode::LtIntTrusted
                            | OpCode::GteIntTrusted
                            | OpCode::LteIntTrusted
                            | OpCode::Add
                            | OpCode::Sub
                            | OpCode::Mul
                            | OpCode::Div
                            | OpCode::Mod
                            | OpCode::Pow
                            | OpCode::Gt
                            | OpCode::Lt
                            | OpCode::Gte
                            | OpCode::Lte
                            | OpCode::Eq
                            | OpCode::Neq => break,
                            OpCode::LoadLocal
                            | OpCode::LoadLocalTrusted
                            | OpCode::LoadModuleBinding
                            | OpCode::PushConst
                            | OpCode::Dup
                            | OpCode::Swap
                            | OpCode::IntToNumber
                            | OpCode::NumberToInt => continue,
                            _ => break,
                        }
                    }
                    if self.in_unboxed_f64_context() {
                        // Float-unboxed context: push raw f64 constant
                        let f64_val = self.builder.ins().f64const(*n);
                        // Push NaN-boxed bits to legacy stack (for SSA variable tracking)
                        let bits = box_number(*n);
                        let boxed_val = self.builder.ins().iconst(types::I64, bits as i64);
                        self.stack_push(boxed_val);
                        self.typed_stack.replace_top(TypedValue::f64(f64_val));
                    } else if self.in_unboxed_int_context()
                        && !consumer_is_typed_float
                        && *n == (*n as i64) as f64
                    {
                        // Int-unboxed context without float locals: exact integer
                        // constant can be kept as raw i64.
                        let raw = self.builder.ins().iconst(types::I64, *n as i64);
                        self.stack_push(raw);
                        self.typed_stack.replace_top(TypedValue::i64(raw));
                    } else {
                        // Push boxed i64 to legacy stack (auto-pushes boxed to typed_stack)
                        let bits = box_number(*n);
                        let boxed_val = self.builder.ins().iconst(types::I64, bits as i64);
                        self.stack_push_typed(boxed_val, StorageHint::Float64);
                        // Upgrade typed_stack entry from boxed to f64 (enables zero-bitcast chains)
                        let f64_val = self.builder.ins().f64const(*n);
                        self.typed_stack.replace_top(TypedValue::f64(f64_val));
                    }
                }
                Constant::Int(i) => {
                    if self.in_unboxed_int_context() {
                        // Unboxed context: push raw i64 directly (no NaN-boxing)
                        let raw = self.builder.ins().iconst(types::I64, *i);
                        self.stack_push(raw);
                        self.typed_stack.replace_top(TypedValue::i64(raw));
                    } else {
                        // JIT uses f64 for all numbers — convert integer to f64
                        let n = *i as f64;
                        let bits = box_number(n);
                        let boxed_val = self.builder.ins().iconst(types::I64, bits as i64);
                        // Track as Int64 — typed opcodes (AddInt, etc.) expect integer inputs
                        self.stack_push_typed(boxed_val, StorageHint::Int64);
                    }
                }
                Constant::UInt(u) => {
                    if self.in_unboxed_int_context() {
                        let raw = self.builder.ins().iconst(types::I64, *u as i64);
                        self.stack_push(raw);
                        self.typed_stack.replace_top(TypedValue::i64(raw));
                    } else {
                        // For u64 > i64::MAX, box as NativeScalar::U64 via raw bits
                        let vw = if *u <= i64::MAX as u64 {
                            shape_value::ValueWord::from_i64(*u as i64)
                        } else {
                            shape_value::ValueWord::from_native_u64(*u)
                        };
                        let bits = vw.raw_bits();
                        let boxed_val = self.builder.ins().iconst(types::I64, bits as i64);
                        self.stack_push_typed(boxed_val, StorageHint::UInt64);
                    }
                }
                Constant::Bool(b) => {
                    let tag = if *b { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE };
                    let boxed_val = self.builder.ins().iconst(types::I64, tag as i64);
                    self.stack_push_typed(boxed_val, StorageHint::Bool);
                    // typed_stack auto-pushed boxed — correct for bool
                }
                Constant::String(s) => {
                    let boxed = jit_box(HK_STRING, s.clone());
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push_typed(boxed_val, StorageHint::String);
                    // typed_stack auto-pushed boxed — correct for string
                }
                Constant::Duration(dur) => {
                    use crate::ast::DurationUnit;
                    let unit_code = match dur.unit {
                        DurationUnit::Seconds => 0u8,
                        DurationUnit::Minutes => 1u8,
                        DurationUnit::Hours => 2u8,
                        DurationUnit::Days => 3u8,
                        DurationUnit::Weeks => 4u8,
                        DurationUnit::Months => 5u8,
                        DurationUnit::Years => 6u8,
                        DurationUnit::Samples => 7u8,
                    };
                    let jit_dur = JITDuration::new(dur.value, unit_code);
                    let boxed = JITDuration::box_duration(jit_dur);
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push(boxed_val);
                }
                Constant::TimeReference(time_ref) => {
                    let boxed = jit_box(HK_TIME, time_ref.clone());
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push(boxed_val);
                }
                Constant::DateTimeExpr(dt_expr) => {
                    let boxed = jit_box(HK_TIME, dt_expr.clone());
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push(boxed_val);
                }
                Constant::DataDateTimeRef(dt_ref) => {
                    let boxed = jit_box(HK_DATA_REFERENCE, dt_ref.clone());
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push(boxed_val);
                }
                Constant::Function(fn_id) => {
                    let boxed = box_function(*fn_id as u16);
                    let boxed_val = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.stack_push(boxed_val);
                }
                _ => {
                    let boxed_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(boxed_val);
                }
            };
        }
        Ok(())
    }

    pub(crate) fn compile_push_null(&mut self) -> Result<(), String> {
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.stack_push(null_val);
        // Upgrade typed_stack: NaN represents null for Option<f64>
        let nan_val = self.builder.ins().f64const(f64::NAN);
        self.typed_stack
            .replace_top(TypedValue::nullable_f64(nan_val));
        Ok(())
    }

    pub(crate) fn compile_pop(&mut self) -> Result<(), String> {
        self.stack_pop(); // auto-pops typed_stack
        Ok(())
    }

    pub(crate) fn compile_dup(&mut self) -> Result<(), String> {
        if let Some(a) = self.stack_peek() {
            // Save typed info before push (peek doesn't pop)
            let tv = self.typed_stack.peek().copied();
            self.stack_push(a); // auto-pushes boxed to typed_stack
            if let Some(tv) = tv {
                self.typed_stack.replace_top(tv);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_swap(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            // Save typed info before popping
            let top_tv = self.typed_stack.peek().copied();
            let top = self.stack_pop().unwrap(); // auto-pops typed_stack
            let sec_tv = self.typed_stack.peek().copied();
            let second = self.stack_pop().unwrap(); // auto-pops typed_stack
            // Push in swapped order: old top goes to bottom, old second on top
            self.stack_push(top); // auto-pushes boxed
            if let Some(tv) = top_tv {
                self.typed_stack.replace_top(tv);
            }
            self.stack_push(second); // auto-pushes boxed
            if let Some(tv) = sec_tv {
                self.typed_stack.replace_top(tv);
            }
        }
        Ok(())
    }
}
