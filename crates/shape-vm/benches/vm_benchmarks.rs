//! Performance governance benchmarks for Shape VM hot paths.
//!
//! ## Acceptance bands (CI gate):
//! - No benchmark regresses >10% from baseline with p<0.05
//! - Trusted ops (when available) must be faster than guarded (p<0.05)
//! - GC young pause p99 tracked but no assumed target until baseline established
//!
//! ## Benchmark groups:
//! 1. `typed_arithmetic` — AddInt/AddNumber guarded vs direct i64/f64 baseline
//! 2. `dispatch_loop` — tight while-loop throughput (compile + execute)
//! 3. `jit_dispatch` — VM-to-JIT-to-VM transition cost (placeholder)
//! 4. `gc_alloc` — BumpAllocator throughput and collection overhead (feature-gated)
//! 5. `gc_young_pause` — young-gen collection pause time (feature-gated)

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use shape_vm::bytecode::{Constant, Operand};
use shape_vm::tier::TierManager;
use shape_vm::{BytecodeProgram, Instruction, OpCode, VMConfig, VirtualMachine};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute a pre-built bytecode program on a fresh VM.
fn execute_program(program: &BytecodeProgram) {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program.clone());
    let _ = black_box(vm.execute(None));
}

// ---------------------------------------------------------------------------
// 1. bench_typed_arithmetic
// ---------------------------------------------------------------------------

/// Build a program that pushes two integer constants, runs AddInt, and halts.
fn build_add_int_program(a: i64, b: i64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let ca = prog.add_constant(Constant::Int(a));
    let cb = prog.add_constant(Constant::Int(b));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(ca)),
    ));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(cb)),
    ));
    prog.emit(Instruction::simple(OpCode::AddInt));
    prog.emit(Instruction::simple(OpCode::Pop));
    prog.emit(Instruction::simple(OpCode::Halt));
    prog
}

/// Build a program that pushes two f64 constants, runs AddNumber, and halts.
fn build_add_number_program(a: f64, b: f64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let ca = prog.add_constant(Constant::Number(a));
    let cb = prog.add_constant(Constant::Number(b));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(ca)),
    ));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(cb)),
    ));
    prog.emit(Instruction::simple(OpCode::AddNumber));
    prog.emit(Instruction::simple(OpCode::Pop));
    prog.emit(Instruction::simple(OpCode::Halt));
    prog
}

/// Build a program that pushes two integer constants, runs AddIntTrusted, and halts.
/// Simulates the bytecode the compiler emits for `let x: int = a; let y: int = b; let z = x + y`
/// where both operands have compiler-proved int types.
fn build_add_int_trusted_program(a: i64, b: i64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let ca = prog.add_constant(Constant::Int(a));
    let cb = prog.add_constant(Constant::Int(b));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(ca)),
    ));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(cb)),
    ));
    prog.emit(Instruction::simple(OpCode::AddIntTrusted));
    prog.emit(Instruction::simple(OpCode::Pop));
    prog.emit(Instruction::simple(OpCode::Halt));
    prog
}

/// Build a program that pushes two f64 constants, runs AddNumberTrusted, and halts.
/// Simulates `let x: number = a; let y: number = b; let z = x + y`.
fn build_add_number_trusted_program(a: f64, b: f64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let ca = prog.add_constant(Constant::Number(a));
    let cb = prog.add_constant(Constant::Number(b));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(ca)),
    ));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(cb)),
    ));
    prog.emit(Instruction::simple(OpCode::AddNumberTrusted));
    prog.emit(Instruction::simple(OpCode::Pop));
    prog.emit(Instruction::simple(OpCode::Halt));
    prog
}

/// Build a fully-trusted variant of the while-loop program.
///
/// Uses trusted opcodes throughout: LoadLocalTrusted, LtIntTrusted,
/// JumpIfFalseTrusted, AddIntTrusted. This is the bytecode the compiler
/// would emit when all locals have typed `let` bindings (e.g., `let x: int = 0`).
///
/// ```text
///   let x: int = 0      // local 0, compiler-proved int
///   while x < N {
///       x = x + 1
///   }
/// ```
///
/// Bytecode layout:
///   0: PushConst(0)             -- push 0
///   1: StoreLocal(0)            -- x = 0
///   2: LoadLocalTrusted(0)      -- [loop top] push x (trusted: proven int)
///   3: PushConst(N)             -- push limit
///   4: LtIntTrusted             -- x < N (trusted: both ints)
///   5: JumpIfFalseTrusted(+5)   -- if false, jump to Halt
///   6: LoadLocalTrusted(0)      -- push x (trusted)
///   7: PushConst(1)             -- push 1
///   8: AddIntTrusted            -- x + 1 (trusted: both ints)
///   9: StoreLocal(0)            -- x = result
///  10: Jump(-8)                 -- back to instruction 2
///  11: Halt
fn build_trusted_loop_program(iterations: i64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let c_zero = prog.add_constant(Constant::Int(0));
    let c_limit = prog.add_constant(Constant::Int(iterations));
    let c_one = prog.add_constant(Constant::Int(1));

    // 0: push 0
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_zero)),
    ));
    // 1: store local 0 (x = 0)
    prog.emit(Instruction::new(
        OpCode::StoreLocal,
        Some(Operand::Local(0)),
    ));
    // 2: load local 0 (trusted — compiler proved int)
    prog.emit(Instruction::new(
        OpCode::LoadLocalTrusted,
        Some(Operand::Local(0)),
    ));
    // 3: push limit
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_limit)),
    ));
    // 4: x < N (trusted — both operands proven int)
    prog.emit(Instruction::simple(OpCode::LtIntTrusted));
    // 5: jump if false to halt (trusted — condition proven bool)
    prog.emit(Instruction::new(
        OpCode::JumpIfFalseTrusted,
        Some(Operand::Offset(5)),
    ));
    // 6: load local 0 (trusted)
    prog.emit(Instruction::new(
        OpCode::LoadLocalTrusted,
        Some(Operand::Local(0)),
    ));
    // 7: push 1
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_one)),
    ));
    // 8: AddIntTrusted (trusted — both operands proven int)
    prog.emit(Instruction::simple(OpCode::AddIntTrusted));
    // 9: store local 0
    prog.emit(Instruction::new(
        OpCode::StoreLocal,
        Some(Operand::Local(0)),
    ));
    // 10: jump back to instruction 2
    prog.emit(Instruction::new(OpCode::Jump, Some(Operand::Offset(-8))));
    // 11: halt
    prog.emit(Instruction::simple(OpCode::Halt));

    prog
}

fn bench_typed_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("typed_arithmetic");

    // --- AddInt (guarded): execute the VM opcode path ---
    let add_int_prog = build_add_int_program(42, 58);
    group.bench_function("add_int_guarded", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&add_int_prog));
            }
        });
    });

    // --- AddInt (direct): raw i64 addition baseline ---
    group.bench_function("add_int_direct", |b| {
        b.iter(|| {
            let mut acc: i64 = 0;
            for i in 0..1000i64 {
                acc = black_box(black_box(acc) + black_box(i));
            }
            black_box(acc);
        });
    });

    // --- AddNumber (guarded): execute the VM opcode path ---
    let add_num_prog = build_add_number_program(3.14, 2.72);
    group.bench_function("add_number_guarded", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&add_num_prog));
            }
        });
    });

    // --- AddNumber (direct): raw f64 addition baseline ---
    group.bench_function("add_number_direct", |b| {
        b.iter(|| {
            let mut acc: f64 = 0.0;
            for i in 0..1000 {
                acc = black_box(black_box(acc) + black_box(i as f64));
            }
            black_box(acc);
        });
    });

    // --- AddIntTrusted: compiler-proved integer operands, no runtime guard ---
    let add_int_trusted_prog = build_add_int_trusted_program(42, 58);
    group.bench_function("add_int_trusted", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&add_int_trusted_prog));
            }
        });
    });

    // --- AddNumberTrusted: compiler-proved float operands, no runtime guard ---
    let add_num_trusted_prog = build_add_number_trusted_program(3.14, 2.72);
    group.bench_function("add_number_trusted", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&add_num_trusted_prog));
            }
        });
    });

    // --- Trusted loop: all-trusted opcode loop (LoadLocalTrusted + LtIntTrusted +
    //     JumpIfFalseTrusted + AddIntTrusted) vs guarded loop ---
    let trusted_loop_prog = build_trusted_loop_program(1_000);
    group.bench_function("loop_1k_trusted", |b| {
        b.iter(|| {
            execute_program(black_box(&trusted_loop_prog));
        });
    });

    let guarded_loop_prog = build_loop_program(1_000);
    group.bench_function("loop_1k_guarded", |b| {
        b.iter(|| {
            execute_program(black_box(&guarded_loop_prog));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. bench_dispatch_loop
// ---------------------------------------------------------------------------

/// Build a tight while-loop program:
///
/// ```text
///   let x = 0           // local 0
///   while x < N {
///       x = x + 1
///   }
/// ```
///
/// Bytecode layout:
///   0: PushConst(0)      — push 0
///   1: StoreLocal(0)     — x = 0
///   2: LoadLocal(0)      — [loop top] push x
///   3: PushConst(N)      — push limit
///   4: LessThan          — x < N
///   5: JumpIfFalse(+5)   — if false, jump to Halt (instruction 10)
///   6: LoadLocal(0)      — push x
///   7: PushConst(1)      — push 1
///   8: AddInt             — x + 1
///   9: StoreLocal(0)     — x = result
///  10: Jump(-8)           — back to instruction 2
///  11: Halt
fn build_loop_program(iterations: i64) -> BytecodeProgram {
    let mut prog = BytecodeProgram::new();
    let c_zero = prog.add_constant(Constant::Int(0));
    let c_limit = prog.add_constant(Constant::Int(iterations));
    let c_one = prog.add_constant(Constant::Int(1));

    // 0: push 0
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_zero)),
    ));
    // 1: store local 0 (x = 0)
    prog.emit(Instruction::new(
        OpCode::StoreLocal,
        Some(Operand::Local(0)),
    ));
    // 2: load local 0 (loop top)
    prog.emit(Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))));
    // 3: push limit
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_limit)),
    ));
    // 4: x < N
    prog.emit(Instruction::simple(OpCode::Lt));
    // 5: jump if false to halt (skip 5 instructions forward: 6,7,8,9,10 -> land on 11)
    prog.emit(Instruction::new(
        OpCode::JumpIfFalse,
        Some(Operand::Offset(5)),
    ));
    // 6: load local 0
    prog.emit(Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))));
    // 7: push 1
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_one)),
    ));
    // 8: AddInt
    prog.emit(Instruction::simple(OpCode::AddInt));
    // 9: store local 0
    prog.emit(Instruction::new(
        OpCode::StoreLocal,
        Some(Operand::Local(0)),
    ));
    // 10: jump back to instruction 2 (offset = -8 from ip after this instruction)
    prog.emit(Instruction::new(OpCode::Jump, Some(Operand::Offset(-8))));
    // 11: halt
    prog.emit(Instruction::simple(OpCode::Halt));

    prog
}

fn bench_dispatch_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_loop");

    // 1K iterations — measures per-iteration dispatch overhead
    let prog_1k = build_loop_program(1_000);
    group.bench_function("while_loop_1k", |b| {
        b.iter(|| {
            execute_program(black_box(&prog_1k));
        });
    });

    // 100K iterations — sustained throughput
    let prog_100k = build_loop_program(100_000);
    group.bench_function("while_loop_100k", |b| {
        b.iter(|| {
            execute_program(black_box(&prog_100k));
        });
    });

    // 1M iterations — long-running loop
    let prog_1m = build_loop_program(1_000_000);
    group.bench_function("while_loop_1m", |b| {
        b.iter(|| {
            execute_program(black_box(&prog_1m));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. bench_gc_alloc (feature-gated on `gc`)
// ---------------------------------------------------------------------------

#[cfg(feature = "gc")]
fn bench_gc_alloc(c: &mut Criterion) {
    use shape_gc::GcHeap;

    let mut group = c.benchmark_group("gc_alloc");

    // --- bump_alloc_throughput: allocate N 64-byte objects ---
    group.bench_function("bump_alloc_1k", |b| {
        b.iter(|| {
            let heap = GcHeap::new();
            for i in 0..1_000u64 {
                let _ = black_box(heap.alloc(black_box([i; 8])));
            }
        });
    });

    group.bench_function("bump_alloc_10k", |b| {
        b.iter(|| {
            let heap = GcHeap::new();
            for i in 0..10_000u64 {
                let _ = black_box(heap.alloc(black_box([i; 8])));
            }
        });
    });

    // --- gc_collect_empty: collection with no garbage ---
    group.bench_function("gc_collect_empty", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            // Allocate a small amount then collect (nothing is garbage since
            // we are not retaining roots — everything is unreachable)
            for i in 0..100u64 {
                let _ = heap.alloc([i; 8]);
            }
            heap.collect(&mut |_trace| {
                // No roots — everything is garbage
            });
            black_box(heap.stats().collections);
        });
    });

    // --- gc_collect_50pct: collection with 50% garbage ---
    // We retain pointers to half the allocations as "roots".
    group.bench_function("gc_collect_50pct", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            let mut retained = Vec::new();
            for i in 0..200u64 {
                let ptr = heap.alloc([i; 8]);
                if i % 2 == 0 {
                    retained.push(ptr as *mut u8);
                }
            }
            heap.collect(&mut |trace| {
                for ptr in &retained {
                    trace(*ptr);
                }
            });
            black_box(heap.stats().collections);
        });
    });

    group.finish();
}

/// Stub for when `gc` feature is not enabled — still registers the group
/// so the bench target compiles unconditionally.
#[cfg(not(feature = "gc"))]
fn bench_gc_alloc(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_alloc");
    group.bench_function("gc_feature_disabled", |b| {
        b.iter(|| {
            // GC benchmarks require `--features gc`. This is a no-op placeholder.
            black_box(42u64);
        });
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// 4. bench_jit_dispatch_roundtrip (placeholder)
// ---------------------------------------------------------------------------

/// JIT dispatch benchmarks measuring the tiered compilation infrastructure.
///
/// Measures three components of the JIT dispatch path:
///   1. `interpreter_baseline` — pure interpreter execution (no tier manager)
///   2. `tier_manager_overhead` — cost of call counting + native code lookup
///      on every function call, even when no JIT compilation occurs
///   3. `tier_manager_osr_check` — cost of loop back-edge counting + OSR
///      table lookup in a hot loop
///
/// These track the fixed overhead the tier manager adds to interpreted code
/// before any actual JIT compilation kicks in. When native code generation
/// is wired up, add a `jit_roundtrip` variant that must not regress vs
/// `interpreter_baseline`.
fn bench_jit_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("jit_dispatch");

    // --- Baseline: interpreter-only execution of a simple add ---
    let mut prog = BytecodeProgram::new();
    let c_a = prog.add_constant(Constant::Int(10));
    let c_b = prog.add_constant(Constant::Int(20));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_a)),
    ));
    prog.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_b)),
    ));
    prog.emit(Instruction::simple(OpCode::AddInt));
    prog.emit(Instruction::simple(OpCode::Pop));
    prog.emit(Instruction::simple(OpCode::Halt));

    group.bench_function("interpreter_baseline", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&prog));
            }
        });
    });

    // --- Tier manager overhead: call counting + native code lookup ---
    // Exercises the hot path of record_call() + get_native_code() that runs
    // on every function call instruction in the interpreter. No actual JIT
    // compilation occurs (no channels connected), but the bookkeeping cost
    // is measured.
    group.bench_function("tier_manager_overhead", |b| {
        b.iter(|| {
            let mut mgr = TierManager::new(16, true);
            for call_idx in 0..1000u16 {
                let func_id = call_idx % 16;
                black_box(mgr.record_call(func_id, None));
                black_box(mgr.get_native_code(func_id));
            }
        });
    });

    // --- Tier manager OSR check: loop back-edge counting + OSR lookup ---
    // Exercises record_loop_iteration() + get_osr_code() that runs on every
    // loop back-edge. Simulates a hot loop in function 0 at bytecode IP 42.
    group.bench_function("tier_manager_osr_check", |b| {
        b.iter(|| {
            let mut mgr = TierManager::new(4, true);
            for _ in 0..1000u32 {
                black_box(mgr.record_loop_iteration(0, 42));
                black_box(mgr.get_osr_code(0, 42));
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 5. bench_gc_young_pause (feature-gated on `gc`)
// ---------------------------------------------------------------------------

/// Young-generation collection pause time.
///
/// Acceptance: p99 pause is tracked; no assumed target until baseline is
/// established. Once baseline exists, regressions >10% at p<0.05 fail CI.
#[cfg(feature = "gc")]
fn bench_gc_young_pause(c: &mut Criterion) {
    use shape_gc::GcHeap;

    let mut group = c.benchmark_group("gc_young_pause");

    // Small young-gen collection (100 objects, ~6.4 KB)
    group.bench_function("young_collect_100", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            for i in 0..100u64 {
                let _ = heap.alloc([i; 8]);
            }
            // Collect with no roots — simulates a young-gen sweep
            heap.collect(&mut |_trace| {});
            black_box(heap.stats().collections);
        });
    });

    // Medium young-gen collection (1000 objects, ~64 KB)
    group.bench_function("young_collect_1k", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            for i in 0..1_000u64 {
                let _ = heap.alloc([i; 8]);
            }
            heap.collect(&mut |_trace| {});
            black_box(heap.stats().collections);
        });
    });

    // Large young-gen collection (10K objects, ~640 KB)
    group.bench_function("young_collect_10k", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            for i in 0..10_000u64 {
                let _ = heap.alloc([i; 8]);
            }
            heap.collect(&mut |_trace| {});
            black_box(heap.stats().collections);
        });
    });

    // Young-gen with 50% survival (simulates realistic workload)
    group.bench_function("young_collect_50pct_survival_1k", |b| {
        b.iter(|| {
            let mut heap = GcHeap::with_threshold(1024);
            let mut live = Vec::new();
            for i in 0..1_000u64 {
                let ptr = heap.alloc([i; 8]);
                if i % 2 == 0 {
                    live.push(ptr as *mut u8);
                }
            }
            heap.collect(&mut |trace| {
                for ptr in &live {
                    trace(*ptr);
                }
            });
            black_box(heap.stats().collections);
        });
    });

    group.finish();
}

#[cfg(not(feature = "gc"))]
fn bench_gc_young_pause(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_young_pause");
    group.bench_function("gc_feature_disabled", |b| {
        b.iter(|| {
            black_box(42u64);
        });
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion wiring
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_typed_arithmetic,
    bench_dispatch_loop,
    bench_jit_dispatch,
    bench_gc_alloc,
    bench_gc_young_pause,
);
criterion_main!(benches);
