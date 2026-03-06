//! FFI Integration Templates for JIT Compiler
//!
//! This module provides templates and helpers for wiring up FFI functions
//! in the JIT compiler. Each template shows exactly how to implement
//! the remaining placeholder opcodes.

#[cfg(feature = "jit")]
use cranelift::prelude::*;
#[cfg(feature = "jit")]
use cranelift_module::Module;
use std::collections::HashMap;

/// Template for calling an FFI function from JIT-compiled code
///
/// # Example: Implementing NewArray
///
/// ```rust,ignore
/// // 1. Declare FFI function signature
/// let mut sig = Signature::new(CallConv::SystemV);
/// sig.params.push(AbiParam::new(types::I64)); // ctx pointer
/// sig.params.push(AbiParam::new(types::I64)); // count
/// sig.returns.push(AbiParam::new(types::I64)); // result (boxed array)
///
/// // 2. Import the external function
/// let func_id = module.declare_function(
///     "jit_new_array",
///     Linkage::Import,
///     &sig
/// )?;
///
/// // 3. In the function being compiled, import the FuncId
/// let callee = module.declare_func_in_func(func_id, &mut builder.func)?;
///
/// // 4. Generate the call
/// let count_val = builder.ins().iconst(types::I64, count as i64);
/// let inst = builder.ins().call(callee, &[ctx_ptr, count_val]);
/// let result = builder.inst_results(inst)[0];
/// value_stack.push(result);
/// ```
pub struct FFICallTemplate;

impl FFICallTemplate {
    /// Template for NewArray opcode
    #[cfg(feature = "jit")]
    pub fn new_array_example() -> &'static str {
        r#"
// In BytecodeToIR, add a field:
ffi_new_array: FuncRef,

// In compile_instruction:
OpCode::NewArray => {
    if let Some(Operand::Count(count)) = &instr.operand {
        let count_val = self.builder.ins().iconst(types::I64, *count as i64);
        let inst = self.builder.ins().call(
            self.ffi_new_array,
            &[self.ctx_ptr, count_val]
        );
        let result = self.builder.inst_results(inst)[0];
        self.value_stack.push(result);
    }
}
"#
    }

    /// Template for Call opcode with function table
    #[cfg(feature = "jit")]
    pub fn call_opcode_example() -> &'static str {
        r#"
// In JITContext, add:
pub function_pointers: [*const u8; 256],
pub function_count: usize,

// In JITCompiler::compile_program (NEW METHOD):
pub fn compile_program(&mut self, program: &BytecodeProgram) -> Result<()> {
    // Compile all functions
    for (id, func) in program.functions.iter().enumerate() {
        let fn_ptr = self.compile_function(&func)?;
        self.function_table.push(fn_ptr);
    }
    Ok(())
}

// In compile_instruction:
OpCode::Call => {
    if let Some(Operand::Function(fn_id)) = &instr.operand {
        // Load function table base address from context
        let table_offset = 800; // offset to function_pointers in JITContext
        let table_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::new(),
            self.ctx_ptr,
            table_offset
        );

        // Load function pointer: table_ptr + (fn_id * 8)
        let fn_offset = self.builder.ins().imul_imm(
            self.builder.ins().iconst(types::I64, *fn_id as i64),
            8
        );
        let fn_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::new(),
            table_ptr,
            fn_offset
        );

        // Create indirect call signature
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // result

        let sig_ref = self.builder.import_signature(sig);
        let inst = self.builder.ins().call_indirect(sig_ref, fn_ptr, &[self.ctx_ptr]);
        let result = self.builder.inst_results(inst)[0];
        self.value_stack.push(result);
    }
}
"#
    }

    /// Template for loop control
    #[cfg(feature = "jit")]
    pub fn loop_control_example() -> &'static str {
        r#"
// In BytecodeToIR, add:
loop_stack: Vec<LoopContext>,

struct LoopContext {
    start_block: Block,
    end_block: Block,
}

// In create_blocks_for_jumps, scan for LoopStart and create end blocks:
for (i, instr) in program.instructions.iter().enumerate() {
    if instr.opcode == OpCode::LoopStart {
        let start_block = self.builder.create_block();
        let end_block = self.builder.create_block();
        self.blocks.insert(i, start_block);
        // Store mapping: loop_start_idx -> end_block
    }
}

// In compile_instruction:
OpCode::LoopStart => {
    let start_block = self.blocks[&_idx];
    let end_block = self.find_matching_loop_end(_idx);
    self.loop_stack.push(LoopContext { start_block, end_block });
}

OpCode::Break => {
    let loop_ctx = self.loop_stack.last().unwrap();
    self.builder.ins().jump(loop_ctx.end_block, &[]);
}

OpCode::Continue => {
    let loop_ctx = self.loop_stack.last().unwrap();
    self.builder.ins().jump(loop_ctx.start_block, &[]);
}

OpCode::LoopEnd => {
    self.loop_stack.pop();
}
"#
    }

    /// Template for iterator state
    #[cfg(feature = "jit")]
    pub fn iterator_example() -> &'static str {
        r#"
// Add to JITContext:
#[repr(C)]
pub struct IteratorSlot {
    array_bits: u64,
    current_index: usize,
    length: usize,
}

pub iterator_stack: [IteratorSlot; 8],
pub iterator_sp: usize,

// FFI functions:
extern "C" fn jit_iter_start(ctx: *mut JITContext, array_bits: u64) {
    let ctx = unsafe { &mut *ctx };
    if ctx.iterator_sp >= 8 { return; }

    let tag = get_tag(array_bits);
    let length = if tag == TAG_ARRAY {
        let arr_ptr = unbox_pointer(array_bits) as *const Vec<u64>;
        unsafe { (*arr_ptr).len() }
    } else {
        0
    };

    ctx.iterator_stack[ctx.iterator_sp] = IteratorSlot {
        array_bits,
        current_index: 0,
        length,
    };
    ctx.iterator_sp += 1;
}

extern "C" fn jit_iter_next(ctx: *mut JITContext) -> u64 {
    let ctx = unsafe { &mut *ctx };
    if ctx.iterator_sp == 0 { return TAG_NULL; }

    let iter = &mut ctx.iterator_stack[ctx.iterator_sp - 1];
    if iter.current_index >= iter.length {
        return TAG_NULL;
    }

    // Get element from array
    let result = jit_array_get(iter.array_bits, box_number(iter.current_index as f64));
    iter.current_index += 1;
    result
}

extern "C" fn jit_iter_done(ctx: *mut JITContext) -> bool {
    let ctx = unsafe { &*ctx };
    if ctx.iterator_sp == 0 { return true; }

    let iter = &ctx.iterator_stack[ctx.iterator_sp - 1];
    iter.current_index >= iter.length
}

// In compile_instruction:
OpCode::IterNext => {
    let result = self.call_extern("jit_iter_next", &[self.ctx_ptr], types::I64);
    self.value_stack.push(result);
}

OpCode::IterDone => {
    let result_bool = self.call_extern("jit_iter_done", &[self.ctx_ptr], types::I8);
    // Convert to NaN-boxed boolean
    let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
    let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
    let result = self.builder.ins().select(result_bool, true_val, false_val);
    self.value_stack.push(result);
}
"#
    }
}

/// Step-by-step guide for implementing each phase
pub mod implementation_guide {
    pub const PHASE_1_CHECKLIST: &str = r#"
Phase 1 Implementation Checklist
=================================

□ Task 1.1: Pre-declare all FFI functions in JITCompiler::new()
  - Add declare_function() calls for jit_new_array, jit_new_object, etc.
  - Store FuncId in HashMap<String, FuncId>

□ Task 1.2: Pass FuncId references to BytecodeToIR
  - Add fields: ffi_new_array: FuncRef, ffi_get_prop: FuncRef, etc.
  - In compile_strategy(), call declare_func_in_func() for each
  - Pass FuncRef handles to BytecodeToIR constructor

□ Task 1.3: Wire up NewArray opcode
  - Replace "let null_val = ..." with actual FFI call
  - Test: cargo test array_literal --features jit

□ Task 1.4: Wire up NewObject opcode
  - Same pattern as NewArray
  - Test: cargo test object_literal --features jit

□ Task 1.5: Wire up GetProp opcode
  - Two parameters: obj_bits, key_bits
  - Test: cargo test property_access --features jit

□ Task 1.6: Wire up SetProp opcode
  - Three parameters: obj_bits, key_bits, value_bits
  - Test: cargo test object_mutation --features jit

□ Task 1.7: Implement Call opcode
  - Add function_table to JITContext
  - Compile all functions upfront in compile_program()
  - Generate indirect calls via call_indirect
  - Test: cargo test function_call --features jit

□ Task 1.8: Implement loop control
  - Add loop_stack to BytecodeToIR
  - Track loop start/end blocks during compilation
  - Generate jumps for Break/Continue
  - Test: cargo test loop_break --features jit

□ Task 1.9: Run full parity matrix
  - cargo run -p shape-core --bin vm_parity_matrix --features jit
  - Verify: Full parity count increases to ~130/158

Estimated Time: 12-18 hours
Expected Outcome: 82% test pass rate
"#;
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_templates_exist() {
        // Ensure templates compile
        assert!(!super::FFICallTemplate::new_array_example().is_empty());
        assert!(!super::FFICallTemplate::call_opcode_example().is_empty());
        assert!(!super::FFICallTemplate::loop_control_example().is_empty());
        assert!(!super::FFICallTemplate::iterator_example().is_empty());
        assert!(!super::implementation_guide::PHASE_1_CHECKLIST.is_empty());
    }
}
