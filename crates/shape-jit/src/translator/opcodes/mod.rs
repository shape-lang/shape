//! Opcode translation for BytecodeToIR
//!
//! Contains the compile_instruction method that translates each bytecode opcode
//! to Cranelift IR. Organized into sub-modules by opcode category.

mod arithmetic;
mod async_ops;
mod builtins;
mod collections;
mod collections_speculation;
mod control_flow;
mod control_flow_array_licm;
mod control_flow_extras;
mod control_flow_loops;
mod control_flow_result_ops;
mod data;
mod functions;
mod generic_ffi;
mod hof_inline;
mod loop_unboxing;
mod references;
mod shape_guards;
mod speculative;
mod stack;
mod typed_objects;
mod variables;

use shape_vm::bytecode::{Instruction, OpCode};

use cranelift::prelude::{InstBuilder, types};

use crate::nan_boxing::*;

use super::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Main dispatch for opcode translation
    pub(crate) fn compile_instruction(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        match instr.opcode {
            // Stack operations
            OpCode::PushConst => self.compile_push_const(instr),
            OpCode::PushNull => self.compile_push_null(),
            OpCode::Pop => self.compile_pop(),
            OpCode::Dup => self.compile_dup(),
            OpCode::Swap => self.compile_swap(),

            // Arithmetic (generic — for string concat, object merge, mixed-type ops)
            OpCode::Add => self.compile_add(),
            OpCode::Sub => self.compile_sub(),
            OpCode::Mul => self.compile_mul(),
            OpCode::Div => self.compile_div(),
            OpCode::Mod => self.compile_mod(),
            OpCode::Neg => self.compile_neg(),
            OpCode::Pow => self.compile_pow(),

            // Typed arithmetic (compiler-guaranteed types — no runtime dispatch)
            OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt => self.compile_int_arith(instr.opcode),
            OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber => self.compile_float_arith(instr.opcode),
            OpCode::AddDecimal
            | OpCode::SubDecimal
            | OpCode::MulDecimal
            | OpCode::DivDecimal
            | OpCode::ModDecimal
            | OpCode::PowDecimal => self.compile_decimal_arith(instr.opcode),

            // Trusted arithmetic (compiler-proved types, no runtime guard — same JIT path)
            OpCode::AddIntTrusted
            | OpCode::SubIntTrusted
            | OpCode::MulIntTrusted
            | OpCode::DivIntTrusted => self.compile_int_arith(instr.opcode),
            OpCode::AddNumberTrusted
            | OpCode::SubNumberTrusted
            | OpCode::MulNumberTrusted
            | OpCode::DivNumberTrusted => self.compile_float_arith(instr.opcode),

            // Trusted comparisons (compiler-proved types — same JIT path as guarded)
            OpCode::GtIntTrusted
            | OpCode::LtIntTrusted
            | OpCode::GteIntTrusted
            | OpCode::LteIntTrusted => self.compile_int_cmp(instr.opcode),
            OpCode::GtNumberTrusted
            | OpCode::LtNumberTrusted
            | OpCode::GteNumberTrusted
            | OpCode::LteNumberTrusted => self.compile_float_cmp(instr.opcode),

            // Comparisons (generic)
            OpCode::Gt => self.compile_gt(),
            OpCode::Lt => self.compile_lt(),
            OpCode::Gte => self.compile_gte(),
            OpCode::Lte => self.compile_lte(),

            // Typed comparisons (compiler-guaranteed types)
            OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::EqInt
            | OpCode::NeqInt => self.compile_int_cmp(instr.opcode),
            OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::EqNumber
            | OpCode::NeqNumber => self.compile_float_cmp(instr.opcode),
            OpCode::GtDecimal | OpCode::LtDecimal | OpCode::GteDecimal | OpCode::LteDecimal => {
                self.compile_decimal_cmp(instr.opcode)
            }
            OpCode::Eq => self.compile_eq(),
            OpCode::Neq => self.compile_neq(),

            // Logical
            OpCode::And => self.compile_and(),
            OpCode::Or => self.compile_or(),
            OpCode::Not => self.compile_not(),

            // Fuzzy comparisons are now desugared to arithmetic at compile time

            // Variables
            OpCode::LoadLocal | OpCode::LoadLocalTrusted => self.compile_load_local(instr),
            OpCode::StoreLocal | OpCode::StoreLocalTyped => self.compile_store_local(instr),
            OpCode::LoadModuleBinding => self.compile_load_global(instr),
            OpCode::StoreModuleBinding | OpCode::StoreModuleBindingTyped => {
                self.compile_store_global(instr)
            }
            OpCode::LoadClosure => self.compile_load_closure(instr),
            OpCode::StoreClosure => self.compile_store_closure(instr),
            OpCode::MakeClosure => self.compile_make_closure(instr),

            // Built-in function calls
            OpCode::BuiltinCall => self.compile_builtin_call(instr, idx),

            // Data operations (generic DataFrame access)
            OpCode::GetDataField => self.compile_get_data_field(instr),
            OpCode::GetDataRow => self.compile_get_data_row(),
            OpCode::NewArray => self.compile_new_array(instr),
            OpCode::NewObject => self.compile_new_object(instr),
            OpCode::GetProp => self.compile_get_prop(instr),
            OpCode::SetProp => self.compile_set_prop(instr),
            OpCode::SetLocalIndex => self.compile_set_local_index(instr),
            OpCode::SetModuleBindingIndex => self.compile_set_module_binding_index(instr),
            OpCode::Length => self.compile_length(),
            OpCode::SliceAccess => self.compile_slice_access(),
            OpCode::ArrayPush => self.compile_array_push(),
            OpCode::ArrayPushLocal => self.compile_array_push_local(instr),
            OpCode::ArrayPop => self.compile_array_pop(),
            OpCode::NullCoalesce => self.compile_null_coalesce(),
            OpCode::MakeRange => self.compile_make_range(),
            // Type-specialized field access (JIT optimization)
            OpCode::GetFieldTyped => self.compile_get_field_typed(instr),
            OpCode::SetFieldTyped => self.compile_set_field_typed(instr),
            OpCode::NewTypedObject => self.compile_new_typed_object(instr),
            OpCode::TypedMergeObject => self.compile_typed_merge_object(instr),

            // Function calls
            OpCode::Call => self.compile_call(instr, idx),
            OpCode::CallValue => self.compile_call_value(idx),
            OpCode::CallMethod => self.compile_call_method(instr, idx),

            // Control flow
            OpCode::Jump => self.compile_jump(instr, idx),
            OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted => {
                self.compile_jump_if_false(instr, idx)
            }
            OpCode::JumpIfTrue => self.compile_jump_if_true(instr, idx),

            // Loop control
            OpCode::LoopStart => self.compile_loop_start(idx),
            OpCode::LoopEnd => self.compile_loop_end(),
            OpCode::Break => self.compile_break(),
            OpCode::Continue => self.compile_continue(),

            // Iterator operations
            OpCode::IterNext => self.compile_iter_next(),
            OpCode::IterDone => self.compile_iter_done(),

            // Exception handling
            OpCode::SetupTry => self.compile_setup_try(instr, idx),
            OpCode::PopHandler => self.compile_pop_handler(),
            OpCode::Throw => self.compile_throw(),
            OpCode::TryUnwrap => self.compile_try_unwrap(),
            OpCode::UnwrapOption => self.compile_unwrap_option(),

            // Pattern matching — dispatch via generic FFI to VM pattern matcher.
            // Bytecode: pops pattern + value, pushes bool result.
            OpCode::Pattern => self.compile_opcode_via_generic_ffi(instr.opcode, 2, true),

            // Timeframe context — fire-and-forget FFI calls (pops 0 or 1, pushes 0)
            OpCode::PushTimeframe => self.compile_opcode_via_generic_ffi(instr.opcode, 1, false),
            OpCode::PopTimeframe => self.compile_opcode_via_generic_ffi(instr.opcode, 0, false),

            OpCode::RunSimulation => self.compile_run_simulation(),
            OpCode::TypeCheck => self.compile_type_check(instr),
            // Return opcodes
            OpCode::Return => self.compile_return(),
            OpCode::ReturnValue => self.compile_return_value(),

            // Halt/Debug
            OpCode::Halt | OpCode::Nop | OpCode::Debug => Ok(()),

            // Async suspension opcodes — FFI calls with suspension check.
            // Yield: pushes 0, no stack effect
            OpCode::Yield => self.compile_opcode_via_generic_ffi(instr.opcode, 0, false),
            // Suspend: pops 0, pushes 0
            OpCode::Suspend => self.compile_opcode_via_generic_ffi(instr.opcode, 0, false),
            // Resume: pops 0, pushes 0
            OpCode::Resume => self.compile_opcode_via_generic_ffi(instr.opcode, 0, false),
            // Poll: pops 0, pushes 1 (event or null)
            OpCode::Poll => self.compile_opcode_via_generic_ffi(instr.opcode, 0, true),
            // AwaitBar/AwaitTick/Await: pops 1 (future/promise), pushes 1 (result)
            OpCode::AwaitBar | OpCode::AwaitTick | OpCode::Await => {
                self.compile_opcode_via_generic_ffi(instr.opcode, 1, true)
            }

            // Event emission — fire-and-forget FFI calls.
            // EmitAlert: pops 1 (alert value), pushes 0
            OpCode::EmitAlert => self.compile_opcode_via_generic_ffi(instr.opcode, 1, false),
            // EmitEvent: pops 1 (event value), pushes 0
            OpCode::EmitEvent => self.compile_opcode_via_generic_ffi(instr.opcode, 1, false),

            // Closure upvalue close — FFI call to finalize upvalue capture.
            // CloseUpvalue: pops 0, pushes 0 (operand has local index)
            OpCode::CloseUpvalue => self.compile_opcode_via_generic_ffi(instr.opcode, 0, false),

            // Type annotation wrapping — FFI call to wrap value.
            // WrapTypeAnnotation: pops 1, pushes 1
            OpCode::WrapTypeAnnotation => {
                self.compile_opcode_via_generic_ffi(instr.opcode, 1, true)
            }

            // Typed column access (Phase 3c: JIT FFI for direct Arrow buffer reads)
            OpCode::LoadColF64 => self.compile_load_col(instr, self.ffi.load_col_f64),
            OpCode::LoadColI64 => self.compile_load_col(instr, self.ffi.load_col_i64),
            OpCode::LoadColBool => self.compile_load_col(instr, self.ffi.load_col_bool),
            OpCode::LoadColStr => self.compile_load_col(instr, self.ffi.load_col_str),

            // Bitwise operations
            OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor | OpCode::BitShl | OpCode::BitShr => {
                self.compile_bitwise_binary(instr.opcode)
            }
            OpCode::BitNot => self.compile_bit_not(),

            // Result type operations — JIT-compiled via FFI
            OpCode::IsOk => self.compile_is_ok(),
            OpCode::IsErr => self.compile_is_err(),
            OpCode::UnwrapOk => self.compile_unwrap_ok(),
            OpCode::UnwrapErr => self.compile_unwrap_err(),

            // Schema binding and error context — semantic no-ops in JIT
            // (type info is tracked statically, error context is metadata-only)
            OpCode::BindSchema | OpCode::ErrorContext => Ok(()),

            // MergeObject — pop 2 objects, push merged via FFI
            OpCode::MergeObject => self.compile_merge_object(),

            // Convert — generic type conversion via FFI
            OpCode::Convert => self.compile_convert(),

            // Async/task operations — compiled to FFI calls
            OpCode::SpawnTask => self.compile_spawn_task(instr),
            OpCode::JoinInit => self.compile_join_init(instr),
            OpCode::JoinAwait => self.compile_join_await(instr),
            OpCode::CancelTask => self.compile_cancel_task(instr),
            OpCode::AsyncScopeEnter => self.compile_async_scope_enter(instr),
            OpCode::AsyncScopeExit => self.compile_async_scope_exit(instr),

            // Trait object operations — FFI calls to VM vtable dispatch.
            // BoxTraitObject: pops 1 (value), pushes 1 (trait object)
            OpCode::BoxTraitObject => self.compile_opcode_via_generic_ffi(instr.opcode, 1, true),
            // DynMethodCall: pops N+1 (receiver + args), pushes 1 (result)
            // Stack effect depends on operand — use generic FFI trampoline
            OpCode::DynMethodCall => self.compile_opcode_via_generic_ffi(instr.opcode, 1, true),

            // Type coercion opcodes
            OpCode::IntToNumber => self.compile_int_to_number(),
            OpCode::NumberToInt => self.compile_number_to_int(),

            // Reference opcodes — pointer-based references for in-place mutation
            OpCode::MakeRef => self.compile_make_ref(instr),
            OpCode::DerefLoad => self.compile_deref_load(instr),
            OpCode::DerefStore => self.compile_deref_store(instr),
            OpCode::SetIndexRef => self.compile_set_index_ref(instr),

            // Drop opcodes — FFI call to Drop::drop on the popped value.
            // Bytecode: pops 1, pushes 0.
            OpCode::DropCall | OpCode::DropCallAsync => {
                self.compile_opcode_via_generic_ffi(instr.opcode, 1, false)
            }

            // Foreign/native ABI calls.
            OpCode::CallForeign => self.compile_call_foreign(instr, idx),

            // Box opcodes — FFI calls to create SharedCell for mutable captures.
            // BoxLocal: pops 0, pushes 0 (operand has local index)
            OpCode::BoxLocal | OpCode::BoxModuleBinding => {
                self.compile_opcode_via_generic_ffi(instr.opcode, 0, false)
            }

            // NewTypedArray has the same stack protocol as NewArray (pop N, push 1 array)
            OpCode::NewTypedArray => self.compile_new_array(instr),

            // NewMatrix — create matrix via FFI (pops rows*cols elements, pushes 1)
            OpCode::NewMatrix => self.compile_opcode_via_generic_ffi(instr.opcode, 0, true),

            // Compact typed opcodes — decode NumericWidth and dispatch to int/float paths
            OpCode::AddTyped
            | OpCode::SubTyped
            | OpCode::MulTyped
            | OpCode::DivTyped
            | OpCode::ModTyped
            | OpCode::CmpTyped => self.compile_typed_arith(instr),

            // Width cast — truncate to target integer width
            OpCode::CastWidth => self.compile_cast_width(instr),
        }
    }
}
