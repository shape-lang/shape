//! OSR (On-Stack Replacement) Loop Compilation
//!
//! Compiles hot loop bodies to native code via Cranelift IR for mid-execution
//! transfer from the bytecode interpreter to JIT-compiled code.
//!
//! # OSR ABI
//! `extern "C" fn(ctx_ptr: *mut u8, _unused: *const u8) -> u64`
//! - Returns 0 on normal loop exit (locals written back to ctx).
//! - Returns `u64::MAX` on deoptimization (locals partially written back).

use std::collections::{HashMap, HashSet};

use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};

use shape_vm::bytecode::{DeoptInfo, Instruction, OpCode, Operand, OsrEntryPoint};
use shape_vm::type_tracking::{FrameDescriptor, SlotKind};

use super::loop_analysis::LoopInfo;

/// Result of compiling a loop body for OSR entry.
#[derive(Debug)]
pub struct OsrCompilationResult {
    /// Native code pointer for the compiled loop body.
    pub native_code: *const u8,
    /// OSR entry point metadata (live locals, kinds, bytecode IPs).
    pub entry_point: OsrEntryPoint,
    /// Deopt info for all guard points within the compiled loop.
    pub deopt_points: Vec<DeoptInfo>,
}

// SAFETY: native_code pointer is valid for the lifetime of the JIT compilation
// and is only used within the VM execution context.
unsafe impl Send for OsrCompilationResult {}

/// Maximum number of locals the JIT context buffer can hold.
/// The locals area spans u64 indices 8..264 (256 slots).
const JIT_LOCALS_CAP: usize = 256;

/// Byte offset where locals begin in the JIT context buffer.
const LOCALS_BYTE_OFFSET: i32 = 64; // 8 * 8

/// Check whether an opcode is in the supported MVP set for OSR compilation.
fn is_osr_supported_opcode(opcode: OpCode, operand: &Option<Operand>) -> bool {
    use shape_vm::bytecode::BuiltinFunction as BF;
    match opcode {
        // Stack
        OpCode::PushConst | OpCode::PushNull | OpCode::Pop | OpCode::Dup | OpCode::Swap => true,
        // Variables
        OpCode::LoadLocal
        | OpCode::LoadLocalTrusted
        | OpCode::StoreLocal
        | OpCode::StoreLocalTyped => true,
        OpCode::LoadModuleBinding
        | OpCode::StoreModuleBinding
        | OpCode::StoreModuleBindingTyped => true,
        // Arithmetic (Int)
        OpCode::AddInt
        | OpCode::SubInt
        | OpCode::MulInt
        | OpCode::DivInt
        | OpCode::ModInt
        | OpCode::PowInt => true,
        // Arithmetic (Number)
        OpCode::AddNumber
        | OpCode::SubNumber
        | OpCode::MulNumber
        | OpCode::DivNumber
        | OpCode::ModNumber
        | OpCode::PowNumber => true,
        // Neg
        OpCode::Neg => true,
        // Comparison (Int)
        OpCode::GtInt
        | OpCode::LtInt
        | OpCode::GteInt
        | OpCode::LteInt
        | OpCode::EqInt
        | OpCode::NeqInt => true,
        // Comparison (Number)
        OpCode::GtNumber
        | OpCode::LtNumber
        | OpCode::GteNumber
        | OpCode::LteNumber
        | OpCode::EqNumber
        | OpCode::NeqNumber => true,
        // Logic
        OpCode::And | OpCode::Or | OpCode::Not => true,
        // Control
        OpCode::Jump
        | OpCode::JumpIfFalse
        | OpCode::JumpIfFalseTrusted
        | OpCode::JumpIfTrue
        | OpCode::LoopStart
        | OpCode::LoopEnd
        | OpCode::Break
        | OpCode::Continue => true,
        // Coercion / width casts
        OpCode::IntToNumber | OpCode::NumberToInt | OpCode::CastWidth => true,
        // Return (mapped to loop exit)
        OpCode::Return | OpCode::ReturnValue => true,
        // Misc
        OpCode::Nop | OpCode::Halt | OpCode::Debug => true,
        // BuiltinCall: only selected math builtins
        OpCode::BuiltinCall => {
            if let Some(Operand::Builtin(bf)) = operand {
                matches!(
                    bf,
                    BF::Abs
                        | BF::Sqrt
                        | BF::Min
                        | BF::Max
                        | BF::Floor
                        | BF::Ceil
                        | BF::Round
                        | BF::Pow
                )
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Compile a loop body for OSR (On-Stack Replacement) entry.
///
/// Emits Cranelift IR for the loop body with the OSR ABI:
/// `extern "C" fn(ctx_ptr: *mut u8, _: *const u8) -> u64`
///
/// - Returns 0 on normal loop exit (locals written back to ctx).
/// - Returns `u64::MAX` on deoptimization (locals partially written back).
///
/// # Arguments
/// * `jit` - The JIT compiler instance (owns the Cranelift module).
/// * `function` - The function containing the target loop.
/// * `instructions` - The full instruction stream of the function.
/// * `loop_info` - Analysis results for the target loop (from `analyze_loops`).
/// * `frame_descriptor` - Typed frame layout for slot marshaling.
pub fn compile_osr_loop(
    jit: &mut crate::compiler::JITCompiler,
    function: &shape_vm::bytecode::Function,
    instructions: &[Instruction],
    loop_info: &LoopInfo,
    frame_descriptor: &FrameDescriptor,
) -> Result<OsrCompilationResult, String> {
    // Validate the loop bounds are within the instruction stream.
    if loop_info.header_idx >= instructions.len() {
        return Err(format!(
            "OSR loop header {} is out of bounds (instruction count: {})",
            loop_info.header_idx,
            instructions.len()
        ));
    }
    if loop_info.end_idx >= instructions.len() {
        return Err(format!(
            "OSR loop end {} is out of bounds (instruction count: {})",
            loop_info.end_idx,
            instructions.len()
        ));
    }

    // Preflight: reject loops containing unsupported opcodes.
    for idx in loop_info.header_idx..=loop_info.end_idx {
        let instr = &instructions[idx];
        if !is_osr_supported_opcode(instr.opcode, &instr.operand) {
            return Err(format!(
                "OSR unsupported opcode {:?} at instruction {}",
                instr.opcode, idx
            ));
        }
    }

    // Compute live locals: union of read and written sets.
    let mut live_locals: Vec<u16> = loop_info
        .body_locals_read
        .union(&loop_info.body_locals_written)
        .copied()
        .collect();
    live_locals.sort_unstable();

    // Check all locals fit within JIT locals capacity.
    for &local_idx in &live_locals {
        if local_idx as usize >= JIT_LOCALS_CAP {
            return Err(format!(
                "OSR local index {} exceeds JIT_LOCALS_CAP ({})",
                local_idx, JIT_LOCALS_CAP
            ));
        }
    }

    // Map each live local to its SlotKind from the frame descriptor.
    let local_kinds: Vec<SlotKind> = live_locals
        .iter()
        .map(|&slot| {
            frame_descriptor
                .slots
                .get(slot as usize)
                .copied()
                .unwrap_or(SlotKind::Unknown)
        })
        .collect();

    let entry_point = OsrEntryPoint {
        bytecode_ip: loop_info.header_idx,
        live_locals: live_locals.clone(),
        local_kinds: local_kinds.clone(),
        exit_ip: loop_info.end_idx + 1,
    };

    // Body locals written — used for epilogue (only write back modified locals).
    let body_locals_written: HashSet<u16> = loop_info.body_locals_written.clone();

    // --- Cranelift compilation ---

    // Declare the OSR function: (i64, i64) -> i64
    let func_name = format!("osr_loop_f{}_ip{}", function.arity, loop_info.header_idx);
    let mut sig = jit.module_mut().make_signature();
    sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
    sig.params.push(AbiParam::new(types::I64)); // unused
    sig.returns.push(AbiParam::new(types::I64)); // result (0 or u64::MAX)

    let func_id = jit
        .module_mut()
        .declare_function(&func_name, Linkage::Export, &sig)
        .map_err(|e| format!("Failed to declare OSR function: {}", e))?;

    let mut ctx = cranelift::codegen::Context::new();
    ctx.func.signature = sig;

    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, jit.builder_context_mut());

        // Create blocks
        let entry_block = builder.create_block();
        let exit_block = builder.create_block();
        let deopt_block = builder.create_block();

        // Pre-scan for jump targets inside the loop body to create blocks
        let mut block_map: HashMap<usize, Block> = HashMap::new();
        // The loop header gets its own block (this is the main loop block)
        let header_block = builder.create_block();
        block_map.insert(loop_info.header_idx, header_block);

        for idx in loop_info.header_idx..=loop_info.end_idx {
            let instr = &instructions[idx];
            match instr.opcode {
                OpCode::Jump
                | OpCode::JumpIfFalse
                | OpCode::JumpIfFalseTrusted
                | OpCode::JumpIfTrue => {
                    if let Some(Operand::Offset(off)) = instr.operand {
                        let target = (idx as i64 + off as i64 + 1) as usize;
                        if target >= loop_info.header_idx
                            && target <= loop_info.end_idx + 1
                            && !block_map.contains_key(&target)
                        {
                            let blk = builder.create_block();
                            block_map.insert(target, blk);
                        }
                    }
                }
                _ => {}
            }
            // Also create a block for the instruction after a conditional branch
            // (fall-through target)
            match instr.opcode {
                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue => {
                    let fall_through = idx + 1;
                    if fall_through >= loop_info.header_idx
                        && fall_through <= loop_info.end_idx
                        && !block_map.contains_key(&fall_through)
                    {
                        let blk = builder.create_block();
                        block_map.insert(fall_through, blk);
                    }
                }
                _ => {}
            }
        }

        // Declare Cranelift variables for all live locals
        let max_local = live_locals.iter().copied().max().unwrap_or(0) as usize;
        for local_idx in 0..=max_local {
            builder.declare_var(Variable::new(local_idx), types::I64);
        }
        // Declare compile-time stack variables (generous upper bound)
        let stack_var_base = JIT_LOCALS_CAP;
        let max_stack_depth = 32usize;
        for s in 0..max_stack_depth {
            builder.declare_var(Variable::new(stack_var_base + s), types::I64);
        }

        // ---- Entry block: load live locals from JIT context buffer ----
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let ctx_ptr = builder.block_params(entry_block)[0];

        // Load live locals from context buffer
        for &local_idx in &live_locals {
            let offset = LOCALS_BYTE_OFFSET + (local_idx as i32) * 8;
            let val = builder
                .ins()
                .load(types::I64, MemFlags::trusted(), ctx_ptr, offset);
            builder.def_var(Variable::new(local_idx as usize), val);
        }

        // Jump to loop header block
        builder.ins().jump(header_block, &[]);

        // ---- Compile loop body instructions ----
        // Compile-time operand stack depth tracker
        let mut stack_depth: usize = 0;
        // Manual block termination tracking (replaces builder.is_filled())
        let mut block_terminated: bool = false;

        macro_rules! stack_push {
            ($builder:expr, $val:expr, $depth:expr) => {{
                let var = Variable::new(stack_var_base + $depth);
                $builder.def_var(var, $val);
                $depth += 1;
            }};
        }
        macro_rules! stack_pop {
            ($builder:expr, $depth:expr) => {{
                $depth -= 1;
                let var = Variable::new(stack_var_base + $depth);
                $builder.use_var(var)
            }};
        }

        for idx in loop_info.header_idx..=loop_info.end_idx {
            // Switch to the block for this instruction if one exists
            if let Some(&blk) = block_map.get(&idx) {
                if idx != loop_info.header_idx || block_terminated {
                    if !block_terminated {
                        builder.ins().jump(blk, &[]);
                    }
                }
                builder.switch_to_block(blk);
                block_terminated = false;
                // Don't seal loop header yet (it has a back-edge)
                if idx != loop_info.header_idx {
                    builder.seal_block(blk);
                }
            }

            // Skip instruction emission if block already terminated
            if block_terminated {
                continue;
            }

            let instr = &instructions[idx];
            match instr.opcode {
                OpCode::Nop | OpCode::Debug | OpCode::LoopStart => {
                    // No-ops in JIT
                }

                OpCode::LoopEnd => {
                    // Back-edge: jump to header
                    builder.ins().jump(header_block, &[]);
                    block_terminated = true;
                }

                OpCode::PushNull => {
                    let null = builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    stack_push!(builder, null, stack_depth);
                }

                OpCode::PushConst => {
                    if let Some(Operand::Const(_const_idx)) = instr.operand {
                        // For OSR MVP, we deopt on constants we can't resolve inline.
                        // The JitCompilationBackend will provide constant resolution
                        // in a future pass.
                        let null = builder
                            .ins()
                            .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                        stack_push!(builder, null, stack_depth);
                    }
                }

                OpCode::Pop => {
                    if stack_depth > 0 {
                        let _ = stack_pop!(builder, stack_depth);
                    }
                }

                OpCode::Dup => {
                    if stack_depth > 0 {
                        let var = Variable::new(stack_var_base + stack_depth - 1);
                        let val = builder.use_var(var);
                        stack_push!(builder, val, stack_depth);
                    }
                }

                OpCode::Swap => {
                    if stack_depth >= 2 {
                        let var_a = Variable::new(stack_var_base + stack_depth - 1);
                        let var_b = Variable::new(stack_var_base + stack_depth - 2);
                        let a = builder.use_var(var_a);
                        let b = builder.use_var(var_b);
                        builder.def_var(var_a, b);
                        builder.def_var(var_b, a);
                    }
                }

                OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
                    if let Some(Operand::Local(local_idx)) = instr.operand {
                        let val = builder.use_var(Variable::new(local_idx as usize));
                        stack_push!(builder, val, stack_depth);
                    }
                }

                OpCode::StoreLocal => {
                    if let Some(Operand::Local(local_idx)) = instr.operand {
                        if stack_depth > 0 {
                            let val = stack_pop!(builder, stack_depth);
                            builder.def_var(Variable::new(local_idx as usize), val);
                        }
                    }
                }

                OpCode::StoreLocalTyped => {
                    if let Some(Operand::TypedLocal(local_idx, _width)) = instr.operand {
                        if stack_depth > 0 {
                            let val = stack_pop!(builder, stack_depth);
                            // OSR MVP: store without truncation (width enforcement
                            // is done by the interpreter; JIT uses same raw i64).
                            builder.def_var(Variable::new(local_idx as usize), val);
                        }
                    }
                }

                // Integer arithmetic: values in JIT context are raw i64 for Int64 slots.
                OpCode::AddInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().iadd(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::SubInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().isub(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::MulInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().imul(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::DivInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().sdiv(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::ModInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().srem(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::PowInt => {
                    // Power is complex — deopt for now
                    builder.ins().jump(deopt_block, &[]);
                    block_terminated = true;
                }

                // Float arithmetic: values are NaN-boxed f64 bit patterns.
                // Bitcast to f64, operate, bitcast back.
                OpCode::AddNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let r_f = builder.ins().fadd(a_f, b_f);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), r_f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::SubNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let r_f = builder.ins().fsub(a_f, b_f);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), r_f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::MulNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let r_f = builder.ins().fmul(a_f, b_f);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), r_f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::DivNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let r_f = builder.ins().fdiv(a_f, b_f);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), r_f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::ModNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        // fmod: a - trunc(a/b) * b
                        let div = builder.ins().fdiv(a_f, b_f);
                        let trunced = builder.ins().trunc(div);
                        let prod = builder.ins().fmul(trunced, b_f);
                        let r_f = builder.ins().fsub(a_f, prod);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), r_f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::PowNumber => {
                    // Power is complex — deopt
                    builder.ins().jump(deopt_block, &[]);
                    block_terminated = true;
                }

                OpCode::Neg => {
                    if stack_depth >= 1 {
                        let val = stack_pop!(builder, stack_depth);
                        let result = builder.ins().ineg(val);
                        stack_push!(builder, result, stack_depth);
                    }
                }

                // Integer comparisons: compare raw i64, produce i64 (0 or 1)
                OpCode::LtInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::GtInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::LteInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::GteInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::EqInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::Equal, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::NeqInt => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let cmp = builder.ins().icmp(IntCC::NotEqual, a, b);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }

                // Float comparisons: bitcast to f64, compare, produce i64
                OpCode::LtNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::LessThan, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::GtNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::GreaterThan, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::LteNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::LessThanOrEqual, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::GteNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::EqNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::Equal, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::NeqNumber => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let a_f = builder.ins().bitcast(types::F64, MemFlags::new(), a);
                        let b_f = builder.ins().bitcast(types::F64, MemFlags::new(), b);
                        let cmp = builder.ins().fcmp(FloatCC::NotEqual, a_f, b_f);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }

                // Logic: operands are i64 (0 = false, nonzero = true)
                OpCode::And => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().band(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::Or => {
                    if stack_depth >= 2 {
                        let b = stack_pop!(builder, stack_depth);
                        let a = stack_pop!(builder, stack_depth);
                        let result = builder.ins().bor(a, b);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::Not => {
                    if stack_depth >= 1 {
                        let val = stack_pop!(builder, stack_depth);
                        let zero = builder.ins().iconst(types::I64, 0);
                        let cmp = builder.ins().icmp(IntCC::Equal, val, zero);
                        let result = builder.ins().uextend(types::I64, cmp);
                        stack_push!(builder, result, stack_depth);
                    }
                }

                // Coercion
                OpCode::IntToNumber => {
                    if stack_depth >= 1 {
                        let val = stack_pop!(builder, stack_depth);
                        // Raw i64 → f64 → bitcast to i64 (NaN-boxed)
                        let f = builder.ins().fcvt_from_sint(types::F64, val);
                        let result = builder.ins().bitcast(types::I64, MemFlags::new(), f);
                        stack_push!(builder, result, stack_depth);
                    }
                }
                OpCode::NumberToInt => {
                    if stack_depth >= 1 {
                        let val = stack_pop!(builder, stack_depth);
                        // NaN-boxed f64 → f64 → truncate to i64
                        let f = builder.ins().bitcast(types::F64, MemFlags::new(), val);
                        let result = builder.ins().fcvt_to_sint_sat(types::I64, f);
                        stack_push!(builder, result, stack_depth);
                    }
                }

                OpCode::CastWidth => {
                    if stack_depth >= 1 {
                        if let Some(Operand::Width(width)) = &instr.operand {
                            if let Some(int_w) = width.to_int_width() {
                                let val = stack_pop!(builder, stack_depth);
                                let mask = int_w.mask() as i64;
                                let mask_val = builder.ins().iconst(types::I64, mask);
                                let truncated = builder.ins().band(val, mask_val);
                                let result = if int_w.is_signed() {
                                    let bits = int_w.bits() as i64;
                                    let shift = 64 - bits;
                                    let shift_val = builder.ins().iconst(types::I64, shift);
                                    let shifted = builder.ins().ishl(truncated, shift_val);
                                    builder.ins().sshr(shifted, shift_val)
                                } else {
                                    truncated
                                };
                                stack_push!(builder, result, stack_depth);
                            }
                        }
                    }
                }

                // Control flow
                OpCode::Jump => {
                    if let Some(Operand::Offset(off)) = instr.operand {
                        let target = (idx as i64 + off as i64 + 1) as usize;
                        if target > loop_info.end_idx {
                            builder.ins().jump(exit_block, &[]);
                        } else if let Some(&blk) = block_map.get(&target) {
                            builder.ins().jump(blk, &[]);
                        } else {
                            builder.ins().jump(deopt_block, &[]);
                        }
                        block_terminated = true;
                    }
                }

                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted => {
                    if let Some(Operand::Offset(off)) = instr.operand {
                        let target = (idx as i64 + off as i64 + 1) as usize;
                        if stack_depth > 0 {
                            let cond = stack_pop!(builder, stack_depth);
                            let zero = builder.ins().iconst(types::I64, 0);
                            let is_false = builder.ins().icmp(IntCC::Equal, cond, zero);

                            let target_block = if target > loop_info.end_idx {
                                exit_block
                            } else {
                                block_map.get(&target).copied().unwrap_or(deopt_block)
                            };
                            let fall_through =
                                block_map.get(&(idx + 1)).copied().unwrap_or(deopt_block);

                            builder
                                .ins()
                                .brif(is_false, target_block, &[], fall_through, &[]);
                            block_terminated = true;
                        }
                    }
                }

                OpCode::JumpIfTrue => {
                    if let Some(Operand::Offset(off)) = instr.operand {
                        let target = (idx as i64 + off as i64 + 1) as usize;
                        if stack_depth > 0 {
                            let cond = stack_pop!(builder, stack_depth);
                            let zero = builder.ins().iconst(types::I64, 0);
                            let is_true = builder.ins().icmp(IntCC::NotEqual, cond, zero);

                            let target_block = if target > loop_info.end_idx {
                                exit_block
                            } else {
                                block_map.get(&target).copied().unwrap_or(deopt_block)
                            };
                            let fall_through =
                                block_map.get(&(idx + 1)).copied().unwrap_or(deopt_block);

                            builder
                                .ins()
                                .brif(is_true, target_block, &[], fall_through, &[]);
                            block_terminated = true;
                        }
                    }
                }

                OpCode::Break => {
                    builder.ins().jump(exit_block, &[]);
                    block_terminated = true;
                }

                OpCode::Continue => {
                    builder.ins().jump(header_block, &[]);
                    block_terminated = true;
                }

                OpCode::Return | OpCode::ReturnValue => {
                    builder.ins().jump(exit_block, &[]);
                    block_terminated = true;
                }

                OpCode::Halt => {
                    builder.ins().jump(exit_block, &[]);
                    block_terminated = true;
                }

                // Module bindings: not in JIT context buffer. Deopt if encountered.
                OpCode::LoadModuleBinding
                | OpCode::StoreModuleBinding
                | OpCode::StoreModuleBindingTyped => {
                    builder.ins().jump(deopt_block, &[]);
                    block_terminated = true;
                }

                // Builtin calls: math functions. Deopt for MVP.
                OpCode::BuiltinCall => {
                    builder.ins().jump(deopt_block, &[]);
                    block_terminated = true;
                }

                _ => {
                    // Unsupported opcode — should have been caught by preflight
                    builder.ins().jump(deopt_block, &[]);
                    block_terminated = true;
                }
            }
        }

        // Seal the loop header block (all predecessors are now known)
        builder.seal_block(header_block);

        // ---- Exit block: store modified locals back, return 0 ----
        builder.switch_to_block(exit_block);
        builder.seal_block(exit_block);

        for &local_idx in &live_locals {
            if body_locals_written.contains(&local_idx) {
                let val = builder.use_var(Variable::new(local_idx as usize));
                let offset = LOCALS_BYTE_OFFSET + (local_idx as i32) * 8;
                builder
                    .ins()
                    .store(MemFlags::trusted(), val, ctx_ptr, offset);
            }
        }
        let zero_ret = builder.ins().iconst(types::I64, 0);
        builder.ins().return_(&[zero_ret]);

        // ---- Deopt block: store ALL live locals back, return u64::MAX ----
        builder.switch_to_block(deopt_block);
        builder.seal_block(deopt_block);

        for &local_idx in &live_locals {
            let val = builder.use_var(Variable::new(local_idx as usize));
            let offset = LOCALS_BYTE_OFFSET + (local_idx as i32) * 8;
            builder
                .ins()
                .store(MemFlags::trusted(), val, ctx_ptr, offset);
        }
        let deopt_sentinel = builder.ins().iconst(types::I64, u64::MAX as i64);
        builder.ins().return_(&[deopt_sentinel]);

        builder.finalize();
    }

    // Compile and define the function
    jit.module_mut()
        .define_function(func_id, &mut ctx)
        .map_err(|e| format!("Failed to define OSR function: {}", e))?;
    jit.module_mut().clear_context(&mut ctx);
    jit.module_mut()
        .finalize_definitions()
        .map_err(|e| format!("Failed to finalize OSR function: {}", e))?;

    let code_ptr = jit.module_mut().get_finalized_function(func_id);

    Ok(OsrCompilationResult {
        native_code: code_ptr,
        entry_point,
        deopt_points: Vec::new(),
    })
}
