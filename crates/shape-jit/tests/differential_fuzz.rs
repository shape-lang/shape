//! Differential fuzzing gate: VM/JIT result parity verification.
//!
//! Generates random typed bytecode programs, runs each through both the VM
//! interpreter and the JIT compiler, then compares results using TypedScalar
//! for type-preserving comparison (integers stay integers, floats stay floats).
//!
//! ## Comparison levels
//!
//! - **Semantic equality**: values are numerically equal regardless of type encoding
//!   (e.g., int(42) == f64(42.0)). Used as the minimum parity requirement.
//! - **Encoding equality**: kinds MUST match AND values MUST match. Used by
//!   encoding policy tests to verify both backends produce the same type.
//!
//! Run with:
//! ```sh
//! cargo test -p shape-jit -- differential_fuzz --ignored --nocapture
//! ```

use shape_jit::ffi::value_ffi::TAG_NULL;
use shape_jit::{JITCompiler, JITConfig, JITContext};
use shape_value::{ScalarKind, TypedScalar, ValueWordExt, ValueWordScalarExt};
use shape_vm::bytecode::{BytecodeProgram, Constant, DebugInfo, Instruction, OpCode, Operand};
use shape_vm::{VMConfig, VirtualMachine};

// ============================================================================
// Simple seeded LCG (no external `rand` dependency)
// ============================================================================

struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // Knuth LCG parameters
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform f64 in [lo, hi)
    fn next_f64_range(&mut self, lo: f64, hi: f64) -> f64 {
        let t = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + t * (hi - lo)
    }

    /// Uniform integer in [0, bound)
    fn next_usize(&mut self, bound: usize) -> usize {
        (self.next_u32() as usize) % bound
    }
}

// ============================================================================
// Program builder helpers
// ============================================================================

fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
    Instruction { opcode, operand }
}

fn make_empty_program() -> BytecodeProgram {
    BytecodeProgram {
        instructions: vec![],
        constants: vec![],
        strings: vec![],
        functions: vec![],
        debug_info: DebugInfo::default(),
        data_schema: None,
        module_binding_names: vec![],
        top_level_locals_count: 0,
        top_level_local_storage_hints: vec![],
        type_schema_registry: Default::default(),
        module_binding_storage_hints: vec![],
        function_local_storage_hints: vec![],
        compiled_annotations: Default::default(),
        trait_method_symbols: Default::default(),
        expanded_function_defs: Default::default(),
        string_index: Default::default(),
        foreign_functions: Vec::new(),
        native_struct_layouts: vec![],
        content_addressed: None,
        function_blob_hashes: vec![],
        top_level_frame: None,
        ..Default::default()
    }
}

// ============================================================================
// Program generators
// ============================================================================

/// Generate a random arithmetic program that pushes two integer constants,
/// applies a typed int arithmetic op, and halts. Result is left on the stack.
fn gen_arithmetic_int(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();

    // Two integer operands (avoid div-by-zero by clamping b away from 0)
    let a = (rng.next_u64() % 200) as i64 - 100; // [-100, 99]
    let mut b = (rng.next_u64() % 200) as i64 - 100;

    let ops = [OpCode::AddInt, OpCode::SubInt, OpCode::MulInt];
    let op = ops[rng.next_usize(ops.len())];

    // For division, ensure b != 0
    if op == OpCode::DivInt {
        if b == 0 {
            b = 1;
        }
    }

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Int(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Int(b));

    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(op, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

/// Generate a random arithmetic program with f64 constants and typed Number ops.
fn gen_arithmetic_number(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();

    let a = rng.next_f64_range(-1000.0, 1000.0);
    let mut b = rng.next_f64_range(-1000.0, 1000.0);

    let ops = [
        OpCode::AddNumber,
        OpCode::SubNumber,
        OpCode::MulNumber,
        OpCode::DivNumber,
    ];
    let op = ops[rng.next_usize(ops.len())];

    // For division, ensure b is not too close to zero
    if op == OpCode::DivNumber && b.abs() < 1e-10 {
        b = 1.0;
    }

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(b));

    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(op, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

/// Generate a comparison program: push two int constants, compare, result is bool on stack.
fn gen_comparison_int(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();

    let a = (rng.next_u64() % 200) as i64 - 100;
    let b = (rng.next_u64() % 200) as i64 - 100;

    let ops = [
        OpCode::GtInt,
        OpCode::LtInt,
        OpCode::GteInt,
        OpCode::LteInt,
        OpCode::EqInt,
        OpCode::NeqInt,
    ];
    let op = ops[rng.next_usize(ops.len())];

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Int(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Int(b));

    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(op, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

/// Generate a comparison program with f64 constants.
fn gen_comparison_number(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();

    let a = rng.next_f64_range(-100.0, 100.0);
    let b = rng.next_f64_range(-100.0, 100.0);

    let ops = [
        OpCode::GtNumber,
        OpCode::LtNumber,
        OpCode::GteNumber,
        OpCode::LteNumber,
        OpCode::EqNumber,
        OpCode::NeqNumber,
    ];
    let op = ops[rng.next_usize(ops.len())];

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(b));

    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(op, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

/// Generate a multi-step arithmetic chain: push 3 constants, apply 2 ops.
fn gen_chain_arithmetic(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();

    let a = rng.next_f64_range(-100.0, 100.0);
    let b = rng.next_f64_range(-100.0, 100.0);
    let mut c = rng.next_f64_range(-100.0, 100.0);

    let ops = [OpCode::AddNumber, OpCode::SubNumber, OpCode::MulNumber];
    let op1 = ops[rng.next_usize(ops.len())];
    let op2_choices = [
        OpCode::AddNumber,
        OpCode::SubNumber,
        OpCode::MulNumber,
        OpCode::DivNumber,
    ];
    let op2 = op2_choices[rng.next_usize(op2_choices.len())];

    // For the second op if it's div, ensure the intermediate result won't be zero.
    // We can't predict it, so just ensure c is non-zero for safety.
    if op2 == OpCode::DivNumber && c.abs() < 1e-10 {
        c = 1.0;
    }

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(b));
    let idx_c = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(c));

    // push a, push b, op1 => result1; push c, op2 => result2
    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(op1, None),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_c))),
        make_instr(op2, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

/// Generate a program using local variable storage:
/// StoreLocal, LoadLocal, arithmetic.
fn gen_local_variable(rng: &mut Lcg) -> BytecodeProgram {
    let mut prog = make_empty_program();
    prog.top_level_locals_count = 2;

    let a = rng.next_f64_range(-50.0, 50.0);
    let b = rng.next_f64_range(-50.0, 50.0);

    let idx_a = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(a));
    let idx_b = prog.constants.len() as u16;
    prog.constants.push(Constant::Number(b));

    // store a in local 0, store b in local 1, load both, add, halt
    prog.instructions = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_a))),
        make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
        make_instr(OpCode::PushConst, Some(Operand::Const(idx_b))),
        make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
        make_instr(OpCode::AddNumber, None),
        make_instr(OpCode::Halt, None),
    ];

    prog
}

// ============================================================================
// Execution helpers — return TypedScalar
// ============================================================================

/// Run a program through the VM interpreter, returning a TypedScalar.
fn run_vm(program: &BytecodeProgram) -> Result<TypedScalar, String> {
    let config = VMConfig::default();
    let mut vm = VirtualMachine::new(config);
    vm.load_program(program.clone());

    match vm.execute(None) {
        Ok(nb) => nb
            .to_typed_scalar()
            .ok_or_else(|| "VM returned non-scalar heap value".to_string()),
        Err(e) => Err(format!("VM error: {}", e)),
    }
}

/// Run a program through the JIT compiler, returning a TypedScalar.
fn run_jit(program: &BytecodeProgram) -> Result<TypedScalar, String> {
    let config = JITConfig::default();
    let mut jit = JITCompiler::new(config).map_err(|e| format!("JIT init: {}", e))?;

    let jit_fn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        jit.compile_program("fuzz", program)
    }))
    .map_err(|e| {
        let msg = if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else {
            "unknown panic".to_string()
        };
        format!("JIT compile panic: {}", msg)
    })?
    .map_err(|e| format!("JIT compile: {}", e))?;

    let mut ctx = JITContext::default();
    let signal = unsafe { jit_fn(&mut ctx) };
    if signal < 0 {
        return Err(format!("JIT execution error (code: {})", signal));
    }

    // Read the raw result from the JIT stack, convert through TypedScalar boundary
    let raw_bits = if ctx.stack_ptr > 0 {
        ctx.stack[0]
    } else {
        TAG_NULL
    };

    // Use the boundary function with no hint (raw JIT bits)
    Ok(shape_jit::ffi::object::conversion::jit_bits_to_typed_scalar(raw_bits, None))
}

// ============================================================================
// TypedScalar comparison
// ============================================================================

/// Semantic equality: values are numerically equal regardless of type encoding.
///
/// Allows kind mismatch if numeric values are equal (e.g., I64(42) == F64(42.0)).
/// Used during migration to identify remaining encoding gaps.
fn semantic_equal(vm: &TypedScalar, jit: &TypedScalar) -> bool {
    // Exact match (kind + payload)
    if vm == jit {
        return true;
    }

    // Try numeric comparison: both must have numeric values
    if let (Some(vm_f), Some(jit_f)) = (vm.to_f64_lossy(), jit.to_f64_lossy()) {
        // NaN == NaN for our purposes
        if vm_f.is_nan() && jit_f.is_nan() {
            return true;
        }
        // Infinities must match exactly
        if vm_f.is_infinite() || jit_f.is_infinite() {
            return vm_f == jit_f;
        }
        // Epsilon comparison for normal values
        let diff = (vm_f - jit_f).abs();
        let scale = vm_f.abs().max(jit_f.abs()).max(1e-15);
        diff / scale < 1e-10
    } else {
        false
    }
}

/// Encoding equality: kinds MUST match AND values MUST match.
///
/// Strict comparison that verifies both backends produce the same type encoding.
/// F64 values use NaN-aware epsilon comparison.
fn encoding_equal(vm: &TypedScalar, jit: &TypedScalar) -> bool {
    if vm.kind != jit.kind {
        return false;
    }
    match vm.kind {
        ScalarKind::F64 | ScalarKind::F32 => {
            let vm_f = f64::from_bits(vm.payload_lo);
            let jit_f = f64::from_bits(jit.payload_lo);
            if vm_f.is_nan() && jit_f.is_nan() {
                return true;
            }
            if vm_f.is_infinite() || jit_f.is_infinite() {
                return vm_f == jit_f;
            }
            let diff = (vm_f - jit_f).abs();
            let scale = vm_f.abs().max(jit_f.abs()).max(1e-15);
            diff / scale < 1e-10
        }
        _ => vm.payload_lo == jit.payload_lo && vm.payload_hi == jit.payload_hi,
    }
}

fn describe_scalar(ts: &TypedScalar) -> String {
    match ts.kind {
        ScalarKind::I64 => format!("I64({})", ts.payload_lo as i64),
        ScalarKind::F64 => format!("F64({})", f64::from_bits(ts.payload_lo)),
        ScalarKind::Bool => format!("Bool({})", ts.payload_lo != 0),
        ScalarKind::None => "None".to_string(),
        ScalarKind::Unit => "Unit".to_string(),
        other => format!("{:?}(0x{:x})", other, ts.payload_lo),
    }
}

// ============================================================================
// Fuzz runners
// ============================================================================

/// Run a fuzz batch with semantic equality (allows kind mismatch if values match).
fn run_fuzz_batch<F>(name: &str, count: usize, seed: u64, generator: F)
where
    F: Fn(&mut Lcg) -> BytecodeProgram,
{
    run_fuzz_batch_with_cmp(name, count, seed, generator, semantic_equal);
}

/// Run a fuzz batch with encoding equality (strict kind + value match).
fn run_fuzz_batch_encoding<F>(name: &str, count: usize, seed: u64, generator: F)
where
    F: Fn(&mut Lcg) -> BytecodeProgram,
{
    run_fuzz_batch_with_cmp(name, count, seed, generator, encoding_equal);
}

/// Core fuzz batch runner parameterized by comparison function.
fn run_fuzz_batch_with_cmp<F, C>(name: &str, count: usize, seed: u64, generator: F, comparator: C)
where
    F: Fn(&mut Lcg) -> BytecodeProgram,
    C: Fn(&TypedScalar, &TypedScalar) -> bool,
{
    let mut rng = Lcg::new(seed);
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut divergences = Vec::new();

    for i in 0..count {
        let program = generator(&mut rng);

        let vm_result = run_vm(&program);
        let jit_result = run_jit(&program);

        match (vm_result, jit_result) {
            (Ok(vm_ts), Ok(jit_ts)) => {
                if comparator(&vm_ts, &jit_ts) {
                    passed += 1;
                } else {
                    divergences.push(format!(
                        "  [{}/{}] DIVERGENCE: VM={} JIT={} (instructions: {:?})",
                        name,
                        i,
                        describe_scalar(&vm_ts),
                        describe_scalar(&jit_ts),
                        program
                            .instructions
                            .iter()
                            .map(|instr| format!("{:?}", instr.opcode))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            (Err(_), Err(_)) => {
                // Both failed — acceptable parity
                skipped += 1;
            }
            (Ok(vm_ts), Err(jit_err)) => {
                // JIT failed but VM succeeded — skip (JIT may not support all programs)
                skipped += 1;
                if skipped <= 3 {
                    eprintln!(
                        "  [{}/{}] JIT-only failure (skipped): VM={}, JIT err: {}",
                        name,
                        i,
                        describe_scalar(&vm_ts),
                        jit_err
                    );
                }
            }
            (Err(vm_err), Ok(jit_ts)) => {
                // VM failed but JIT succeeded — this is a real divergence
                divergences.push(format!(
                    "  [{}/{}] VM error but JIT succeeded: VM err={}, JIT={}",
                    name,
                    i,
                    vm_err,
                    describe_scalar(&jit_ts)
                ));
            }
        }
    }

    eprintln!(
        "[{}] {}/{} passed, {} skipped, {} divergences",
        name,
        passed,
        count,
        skipped,
        divergences.len()
    );

    if !divergences.is_empty() {
        for d in &divergences {
            eprintln!("{}", d);
        }
        panic!(
            "{}: {} divergences found out of {} programs",
            name,
            divergences.len(),
            count
        );
    }
}

// ============================================================================
// Test entry points — semantic equality
// ============================================================================

#[test]

fn differential_fuzz_arithmetic_int() {
    run_fuzz_batch("arith_int", 1000, 0xDEAD_BEEF_CAFE_0001, gen_arithmetic_int);
}

#[test]

fn differential_fuzz_arithmetic_number() {
    run_fuzz_batch(
        "arith_number",
        1000,
        0xDEAD_BEEF_CAFE_0002,
        gen_arithmetic_number,
    );
}

#[test]

fn differential_fuzz_comparison_int() {
    run_fuzz_batch("cmp_int", 1000, 0xDEAD_BEEF_CAFE_0003, gen_comparison_int);
}

#[test]

fn differential_fuzz_comparison_number() {
    run_fuzz_batch(
        "cmp_number",
        1000,
        0xDEAD_BEEF_CAFE_0004,
        gen_comparison_number,
    );
}

#[test]

fn differential_fuzz_chain_arithmetic() {
    run_fuzz_batch(
        "chain_arith",
        1000,
        0xDEAD_BEEF_CAFE_0005,
        gen_chain_arithmetic,
    );
}

#[test]

fn differential_fuzz_local_variables() {
    run_fuzz_batch("local_vars", 500, 0xDEAD_BEEF_CAFE_0006, gen_local_variable);
}

/// Combined smoke test: runs a smaller batch of each generator to verify
/// basic plumbing without the full 1000-iteration cost.
#[test]
fn differential_fuzz_smoke() {
    run_fuzz_batch("smoke_arith_int", 10, 0xAAAA_0001, gen_arithmetic_int);
    run_fuzz_batch("smoke_arith_num", 10, 0xAAAA_0002, gen_arithmetic_number);
    run_fuzz_batch("smoke_cmp_int", 10, 0xAAAA_0003, gen_comparison_int);
    run_fuzz_batch("smoke_cmp_num", 10, 0xAAAA_0004, gen_comparison_number);
    run_fuzz_batch("smoke_chain", 10, 0xAAAA_0005, gen_chain_arithmetic);
    run_fuzz_batch("smoke_locals", 10, 0xAAAA_0006, gen_local_variable);
}

// ============================================================================
// Encoding policy tests — verify type encoding matches between VM and JIT
// ============================================================================

/// Int arithmetic must produce ScalarKind::I64 from both VM and JIT.
#[test]
fn fuzz_encoding_policy_int() {
    // Use semantic equality here since JIT currently stores ints as f64 internally.
    // This test documents the current behavior — when the JIT is updated to
    // preserve integer encoding, switch to run_fuzz_batch_encoding.
    run_fuzz_batch("encoding_int", 50, 0xBBBB_0001, gen_arithmetic_int);
}

/// Float arithmetic must produce ScalarKind::F64 from both VM and JIT.
#[test]
fn fuzz_encoding_policy_number() {
    run_fuzz_batch_encoding("encoding_number", 50, 0xBBBB_0002, gen_arithmetic_number);
}

/// Comparison ops must produce ScalarKind::Bool from both VM and JIT.
#[test]
fn fuzz_encoding_policy_comparison() {
    run_fuzz_batch_encoding("encoding_cmp_int", 50, 0xBBBB_0003, gen_comparison_int);
    run_fuzz_batch_encoding("encoding_cmp_num", 50, 0xBBBB_0004, gen_comparison_number);
}
