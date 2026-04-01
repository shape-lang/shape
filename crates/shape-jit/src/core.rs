//! JIT compiler using Cranelift for native code generation
//!
//! Compiles Shape bytecode to native x86-64/ARM machine code for
//! high-performance strategy execution in backtesting.
//!
//! Target: 0.1-1µs per row (vs 2-10µs with VM, 10-50µs with interpreter)
//!
//! # Supported Features
//!
//! **Tier 1 (Fully Implemented):**
//! - Stack operations: PushConst, Pop, Dup, Swap
//! - Arithmetic: Add, Sub, Mul, Div, Mod, Neg, Pow
//! - Comparisons: Gt, Lt, Gte, Lte, Eq, Neq
//! - Logical: And, Or, Not
//! - Variables: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding
//! - Control flow: Jump, JumpIfFalse, JumpIfTrue
//! - Fuzzy comparisons: FuzzyEq, FuzzyGt, FuzzyLt
//! - Data access: DataProp, LoadData (current row only)
//! - Math builtins: abs, sqrt, floor, ceil, round, min, max
//!
//! **Tier 2 (Not Yet Implemented - Requires Heap Allocation):**
//! - Function calls: Call, CallValue (would need function table + calling convention)
//! - Arrays/Objects: NewArray, NewObject, GetProp, SetProp (need heap allocation)
//! - Closures: LoadClosure, MakeClosure (need heap + captured variables)
//! - Advanced loops: Break, Continue (need loop context stack)
//! - Iterators: IterNext, IterDone (need iterator state)
//! - Exceptions: SetupTry, Throw (need unwinding machinery)
//!
//! # Architecture Limitation
//!
//! The current JIT uses a **pure numeric model** (f64-only stack) for maximum
//! performance. This enables ~1µs/row for numeric strategies but cannot support
//! heap-allocated types (arrays, objects, strings, closures).
//!
//! **Strategies that JIT-compile:**
//! - Pure numeric calculations with data rows
//! - Technical indicators (SMA, EMA, RSI)
//! - Entry/exit logic with fuzzy comparisons
//! - Simple control flow (if/while with numeric conditions)
//!
//! **Strategies that fallback to VM:**
//! - Functions calling other functions
//! - Array/object manipulation
//! - Complex data structures
//! - Exception handling
//!
//! Use `can_jit_compile()` and `get_unsupported_opcodes()` to check compatibility.
//!
//! # Examples
//!
//! **✅ JIT-Compatible Strategy:**
//! ```shape
//! function simple_momentum() {
//!     let close = data[0].close;
//!     let prev_close = data[-1].close;
//!     let change_pct = (close - prev_close) / prev_close;
//!
//!     if change_pct > 0.01 {
//!         return "buy";  // Returns signal code 1
//!     }
//!     if change_pct < -0.01 {
//!         return "sell"; // Returns signal code 2
//!     }
//!     return null;      // Returns signal code 0
//! }
//! ```
//! **Bytecode:** Pure numeric, simple branches → **JIT compiles to ~1µs/row**
//!
//! **❌ Non-JIT Strategy (falls back to VM):**
//! ```shape
//! function complex_strategy() {
//!     let prices = [data[-2].close, data[-1].close, data[0].close];
//!     let sma_val = sma(prices, 3);  // Calls a function
//!     return prices[0] > sma_val ? "buy" : null;  // Uses arrays
//! }
//! ```
//! **Bytecode:** Contains Call, NewArray, GetProp → **Runs on VM at ~5µs/row**
//!
//! **Usage:**
//! ```ignore
//! use shape_core::vm::jit::{can_jit_compile, get_unsupported_opcodes};
//!
//! if can_jit_compile(&bytecode) {
//!     engine.compile_strategy_jit(&strategy_fn)?;
//!     engine.set_execution_mode(ExecutionMode::JIT);  // 1µs/row
//! } else {
//!     let blockers = get_unsupported_opcodes(&bytecode);
//!     println!("JIT blocked by: {:?}", blockers);
//!     engine.compile_strategy(&strategy_fn)?;
//!     engine.set_execution_mode(ExecutionMode::BytecodeVM);  // 5µs/row
//! }
//! ```

// Re-export from compiler module
#[allow(unused_imports)]
pub use super::compiler::JITCompiler;
#[allow(unused_imports)]
pub use super::compiler::{can_jit_compile, get_incomplete_opcodes, get_unsupported_opcodes};
#[allow(unused_imports)]
pub use super::context::JITConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JITContext;
    use crate::nan_boxing::{is_number, unbox_number};
    use shape_vm::bytecode::{BytecodeProgram, Constant, Instruction, OpCode, Operand};
    use shape_vm::type_tracking::StorageHint;

    fn run_program_get_number(name: &str, program: BytecodeProgram) -> f64 {
        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program(name, &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };
        assert_eq!(signal, 0);
        assert!(ctx.stack_ptr > 0);
        let result = ctx.stack[0];
        assert!(
            is_number(result),
            "expected numeric result, got bits={result:#x}"
        );
        unbox_number(result)
    }

    fn run_program_get_raw(name: &str, program: BytecodeProgram) -> u64 {
        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program(name, &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };
        assert_eq!(signal, 0);
        assert!(ctx.stack_ptr > 0);
        ctx.stack[0]
    }

    #[test]
    fn test_jit_arithmetic() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
                Instruction::simple(OpCode::Add),
            ],
            constants: vec![Constant::Number(10.0), Constant::Number(5.0)],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile("test_add", &program).unwrap();

        let mut stack = [0.0f64; 100];
        let constants = [10.0f64, 5.0f64];
        let result = unsafe { func(stack.as_mut_ptr(), constants.as_ptr(), 0) };

        assert_eq!(result, 15.0);
    }

    #[test]
    fn test_jit_comparison() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
                Instruction::simple(OpCode::Gt),
            ],
            constants: vec![Constant::Number(10.0), Constant::Number(5.0)],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile("test_gt", &program).unwrap();

        let mut stack = [0.0f64; 100];
        let constants = [10.0f64, 5.0f64];
        let result = unsafe { func(stack.as_mut_ptr(), constants.as_ptr(), 0) };

        assert_eq!(result, 1.0); // true
    }

    #[test]
    fn test_jit_context() {
        // Create column data (generic 5-column schema)
        let col0 = vec![94.0, 95.0, 96.0, 97.0, 98.0, 99.0];
        let col1 = vec![96.0, 97.0, 98.0, 99.0, 100.0, 105.0];
        let col2 = vec![93.0, 94.0, 95.0, 96.0, 97.0, 95.0];
        let col3 = vec![95.0, 96.0, 97.0, 98.0, 99.0, 100.0];
        let col4 = vec![1000.0, 1100.0, 1200.0, 1300.0, 1400.0, 1500.0];

        // Create column pointers array
        let column_ptrs: Vec<*const f64> = vec![
            col0.as_ptr(),
            col1.as_ptr(),
            col2.as_ptr(),
            col3.as_ptr(),
            col4.as_ptr(),
        ];

        let mut ctx = JITContext::default();
        ctx.column_ptrs = column_ptrs.as_ptr();
        ctx.column_count = 5;
        ctx.row_count = col3.len();
        ctx.current_row = 5;

        // Test column access at current row (column 3)
        assert_eq!(ctx.get_column_value(3, 0), 100.0); // current row, col 3
        assert_eq!(ctx.get_column_value(3, -1), 99.0); // previous row, col 3
        assert_eq!(ctx.get_column_value(3, -5), 95.0); // 5 rows ago, col 3
    }

    // ========================================================================
    // Simulation Kernel ABI Tests
    // ========================================================================

    #[test]
    fn test_simulation_kernel_config() {
        use crate::context::SimulationKernelConfig;

        let config = SimulationKernelConfig::new(1, 5)
            .map_column("open", 0)
            .map_column("high", 1)
            .map_column("low", 2)
            .map_column("close", 3)
            .map_column("volume", 4)
            .map_state_field("cash", 0)
            .map_state_field("position", 8)
            .map_state_field("entry_price", 16);

        assert_eq!(config.get_column_index("close"), Some(3));
        assert_eq!(config.get_column_index("volume"), Some(4));
        assert_eq!(config.get_column_index("unknown"), None);

        assert_eq!(config.get_state_offset("cash"), Some(0));
        assert_eq!(config.get_state_offset("position"), Some(8));
        assert_eq!(config.get_state_offset("unknown"), None);
    }

    #[test]
    fn test_simulation_kernel_compilation() {
        use crate::context::SimulationKernelConfig;

        // Create a minimal bytecode program
        let program = BytecodeProgram {
            instructions: vec![Instruction::new(OpCode::PushConst, Some(Operand::Const(0)))],
            constants: vec![Constant::Number(0.0)],
            ..Default::default()
        };

        let config = SimulationKernelConfig::new(1, 5)
            .map_column("close", 3)
            .map_state_field("cash", 0);

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let kernel = jit
            .compile_simulation_kernel("test_kernel", &program, &config)
            .unwrap();

        // Create test data
        let col0 = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let col1 = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let col2 = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let col3 = vec![1000.0, 2000.0, 3000.0, 4000.0, 5000.0]; // "close"
        let col4 = vec![10000.0, 20000.0, 30000.0, 40000.0, 50000.0];

        let series_ptrs: Vec<*const f64> = vec![
            col0.as_ptr(),
            col1.as_ptr(),
            col2.as_ptr(),
            col3.as_ptr(),
            col4.as_ptr(),
        ];

        // Create state buffer (simulating TypedObject)
        let mut state = [0u8; 64];

        // Call kernel at different cursor positions
        for cursor_idx in 0..5 {
            let result = unsafe { kernel(cursor_idx, series_ptrs.as_ptr(), state.as_mut_ptr()) };
            assert_eq!(result, 0, "Kernel should return 0 (continue)");
        }
    }

    #[test]
    fn test_simulation_kernel_direct_data_access() {
        // Test the data access pattern that the kernel enables:
        // series_ptrs[col_idx][cursor_idx]
        let col0 = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let col1 = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let col2 = vec![1000.0, 2000.0, 3000.0, 4000.0, 5000.0];

        let series_ptrs: Vec<*const f64> = vec![col0.as_ptr(), col1.as_ptr(), col2.as_ptr()];

        // Direct memory access pattern (what kernel generates):
        // value = series_ptrs[col_idx][cursor_idx]
        unsafe {
            let col_ptr = *series_ptrs.as_ptr().add(1); // column 1
            let value = *col_ptr.add(2); // cursor index 2
            assert_eq!(value, 300.0);

            // Another access
            let col_ptr = *series_ptrs.as_ptr().add(2); // column 2
            let value = *col_ptr.add(4); // cursor index 4
            assert_eq!(value, 5000.0);
        }
    }

    #[test]
    fn test_kernel_mode_throughput() {
        use crate::context::SimulationKernelConfig;
        use std::time::Instant;

        // Create bytecode that just returns 0 (minimal kernel)
        // This tests the kernel compilation and call overhead
        let program = BytecodeProgram {
            instructions: vec![Instruction::new(OpCode::PushConst, Some(Operand::Const(0)))],
            constants: vec![Constant::Number(0.0)],
            ..Default::default()
        };

        let config = SimulationKernelConfig::new(1, 5)
            .map_column("close", 3)
            .map_state_field("sum", 0);

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let kernel = jit
            .compile_simulation_kernel("bench_kernel", &program, &config)
            .unwrap();

        // Create test data (1M ticks)
        const NUM_TICKS: usize = 1_000_000;
        let col_data: Vec<f64> = (0..NUM_TICKS).map(|i| 100.0 + (i as f64) * 0.01).collect();
        let column_ptrs: Vec<*const f64> = vec![
            col_data.as_ptr(),
            col_data.as_ptr(),
            col_data.as_ptr(),
            col_data.as_ptr(), // col 3 = close
            col_data.as_ptr(),
        ];

        let series_ptrs = column_ptrs.as_ptr();
        let mut state = [0u8; 64];

        // Warm up
        for i in 0..1000 {
            unsafe { kernel(i, series_ptrs, state.as_mut_ptr()) };
        }

        // Benchmark
        let start = Instant::now();
        for cursor_idx in 0..NUM_TICKS {
            let result = unsafe { kernel(cursor_idx, series_ptrs, state.as_mut_ptr()) };
            if result != 0 {
                break;
            }
        }
        let elapsed = start.elapsed();

        let ticks_per_sec = NUM_TICKS as f64 / elapsed.as_secs_f64();
        println!("\n=== Kernel Mode Throughput ===");
        println!("Ticks: {}", NUM_TICKS);
        println!("Time: {:?}", elapsed);
        println!("Throughput: {:.2}M ticks/sec", ticks_per_sec / 1_000_000.0);

        // Should be > 10M ticks/sec for a minimal kernel
        assert!(
            ticks_per_sec > 5_000_000.0,
            "Kernel throughput ({:.2}M/sec) below expected 5M/sec minimum",
            ticks_per_sec / 1_000_000.0
        );
    }

    #[test]
    fn test_correlated_kernel_config() {
        use crate::context::SimulationKernelConfig;

        // Create multi-series config
        let config = SimulationKernelConfig::new_multi_table(1, 3)
            .map_series("spy", 0)
            .map_series("vix", 1)
            .map_series("bonds", 2)
            .map_state_field("position", 0)
            .map_state_field("cash", 8);

        // Verify series mappings
        assert!(config.is_multi_table());
        assert_eq!(config.table_count, 3);
        assert_eq!(config.get_series_index("spy"), Some(0));
        assert_eq!(config.get_series_index("vix"), Some(1));
        assert_eq!(config.get_series_index("bonds"), Some(2));
        assert_eq!(config.get_series_index("unknown"), None);

        // Verify state field mappings
        assert_eq!(config.get_state_offset("position"), Some(0));
        assert_eq!(config.get_state_offset("cash"), Some(8));
    }

    #[test]
    fn test_correlated_kernel_compilation() {
        use crate::context::SimulationKernelConfig;

        // Create minimal bytecode
        let program = BytecodeProgram {
            instructions: vec![Instruction::new(OpCode::PushConst, Some(Operand::Const(0)))],
            constants: vec![Constant::Number(0.0)],
            ..Default::default()
        };

        // Multi-series config: spy and vix
        let config = SimulationKernelConfig::new_multi_table(1, 2)
            .map_series("spy", 0)
            .map_series("vix", 1)
            .map_state_field("position", 0);

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let kernel = jit
            .compile_correlated_kernel("test_correlated", &program, &config)
            .unwrap();

        // Create test data: 2 series with 5 data points each
        let spy_data = vec![100.0, 101.0, 102.0, 103.0, 104.0];
        let vix_data = vec![15.0, 16.0, 25.0, 30.0, 20.0];

        let series_ptrs: Vec<*const f64> = vec![spy_data.as_ptr(), vix_data.as_ptr()];

        let mut state = [0u8; 64];

        // Call kernel at each cursor position
        for cursor_idx in 0..5 {
            let result = unsafe {
                kernel(
                    cursor_idx,
                    series_ptrs.as_ptr(),
                    2, // table_count
                    state.as_mut_ptr(),
                )
            };
            assert_eq!(result, 0, "Correlated kernel should return 0 (continue)");
        }
    }

    #[test]
    fn test_correlated_kernel_direct_series_access() {
        // Test the data access pattern for correlated kernels:
        // series_ptrs[series_idx][cursor_idx]
        let spy_data = vec![100.0, 101.0, 102.0, 103.0, 104.0];
        let vix_data = vec![15.0, 16.0, 25.0, 30.0, 20.0];
        let bonds_data = vec![95.0, 96.0, 97.0, 98.0, 99.0];

        let series_ptrs: Vec<*const f64> =
            vec![spy_data.as_ptr(), vix_data.as_ptr(), bonds_data.as_ptr()];

        // Direct memory access pattern (what correlated kernel generates):
        // value = series_ptrs[series_idx][cursor_idx]
        unsafe {
            // Access spy at cursor 2
            let spy_ptr = *series_ptrs.as_ptr().add(0);
            let spy_val = *spy_ptr.add(2);
            assert_eq!(spy_val, 102.0);

            // Access vix at cursor 3
            let vix_ptr = *series_ptrs.as_ptr().add(1);
            let vix_val = *vix_ptr.add(3);
            assert_eq!(vix_val, 30.0);

            // Access bonds at cursor 4
            let bonds_ptr = *series_ptrs.as_ptr().add(2);
            let bonds_val = *bonds_ptr.add(4);
            assert_eq!(bonds_val, 99.0);
        }
    }

    // ========================================================================
    // Regression Tests: Inline Array Access (Vec Layout Bug Fix)
    // ========================================================================

    /// Regression test: inline array element access via jit_array_info.
    ///
    /// Previously, the JIT assumed Vec<u64> had layout {data_ptr(0), len(8), cap(16)}
    /// but Rust's actual layout is unstable and was {cap(0), data_ptr(8), len(16)}.
    /// This caused segfaults when accessing array elements inline.
    ///
    /// Fixed by using jit_array_info FFI which calls Vec's stable API (as_ptr(), len())
    /// and returns a #[repr(C)] ArrayInfo struct.
    #[test]
    fn test_jit_inline_array_access() {
        use crate::nan_boxing::{is_number, unbox_number};

        // Bytecode: create [10, 20, 30], access element at index 1 → expect 20.0
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 20.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 30.0
                Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),  // [10, 20, 30]
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // index 1.0
                Instruction::simple(OpCode::GetProp),                         // arr[1]
            ],
            constants: vec![
                Constant::Number(10.0),
                Constant::Number(20.0),
                Constant::Number(30.0),
                Constant::Number(1.0),
            ],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program("test_array_access", &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };

        assert_eq!(signal, 0, "JIT execution should succeed");
        assert!(ctx.stack_ptr > 0, "Should have a result on stack");
        let result = ctx.stack[0];
        assert!(is_number(result), "Result should be a number");
        assert_eq!(unbox_number(result), 20.0, "arr[1] should be 20.0");
    }

    /// Regression test: inline array access with negative index.
    ///
    /// Tests that arr[-1] correctly returns the last element.
    /// This exercises the same jit_array_info + inline bounds check path.
    #[test]
    fn test_jit_inline_array_negative_index() {
        use crate::nan_boxing::{is_number, unbox_number};

        // Bytecode: create [10, 20, 30], access arr[-1] → expect 30.0
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
                Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // -1.0
                Instruction::simple(OpCode::GetProp),
            ],
            constants: vec![
                Constant::Number(10.0),
                Constant::Number(20.0),
                Constant::Number(30.0),
                Constant::Number(-1.0),
            ],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program("test_array_neg_idx", &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };

        assert_eq!(signal, 0);
        assert!(ctx.stack_ptr > 0);
        let result = ctx.stack[0];
        assert!(is_number(result));
        assert_eq!(unbox_number(result), 30.0, "arr[-1] should be 30.0");
    }

    // ========================================================================
    // Regression Tests: Reference Opcodes (Stack Slot Bug Fix)
    // ========================================================================

    /// Regression test: MakeRef + DerefStore + DerefLoad.
    ///
    /// Previously, MakeRef stored references as ctx.locals[] addresses, but
    /// compile_direct_call saves/restores ctx.locals[0..arg_count], clobbering
    /// the referenced slot. Fixed by using Cranelift stack slots which live in
    /// the native function's stack frame.
    #[test]
    fn test_jit_reference_deref_store_load() {
        use crate::nan_boxing::{is_number, unbox_number};

        // Bytecode: let x = 42; let ref = &x; *ref = 100; return *ref → expect 100.0
        let program = BytecodeProgram {
            instructions: vec![
                // let x = 42.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 42.0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                // let ref = &x (MakeRef pushes address of local 0's stack slot)
                Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                // *ref = 100.0 (DerefStore writes through the reference)
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 100.0
                Instruction::new(OpCode::DerefStore, Some(Operand::Local(1))),
                // return *ref (DerefLoad reads through the reference)
                Instruction::new(OpCode::DerefLoad, Some(Operand::Local(1))),
            ],
            constants: vec![Constant::Number(42.0), Constant::Number(100.0)],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program("test_ref_deref", &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };

        assert_eq!(signal, 0, "JIT execution should succeed");
        assert!(ctx.stack_ptr > 0, "Should have a result on stack");
        let result = ctx.stack[0];
        assert!(is_number(result), "Result should be a number");
        assert_eq!(
            unbox_number(result),
            100.0,
            "DerefLoad after DerefStore should return the written value"
        );
    }

    /// Regression test: SetIndexRef for array mutation through reference.
    ///
    /// Tests the full reference-based array mutation path:
    /// let arr = [10, 20, 30]; let ref = &arr; ref[1] = 99; return arr[1]
    /// The result should be 99.0 (mutated in-place through reference).
    #[test]
    fn test_jit_set_index_ref() {
        use crate::nan_boxing::{is_number, unbox_number};

        // Bytecode:
        // let arr = [10, 20, 30]
        // let ref = &arr
        // ref[1] = 99
        // return arr[1]  → expect 99.0
        let program = BytecodeProgram {
            instructions: vec![
                // Create array [10, 20, 30]
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 20.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 30.0
                Instruction::new(OpCode::NewArray, Some(Operand::Count(3))),
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                // MakeRef(&arr) → local 1
                Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                // SetIndexRef: ref[1] = 99.0
                // Stack needs: index, value (SetIndexRef pops value first, then index)
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // index 1.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // value 99.0
                Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
                // Read arr[1] to verify mutation
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // index 1.0
                Instruction::simple(OpCode::GetProp),
            ],
            constants: vec![
                Constant::Number(10.0),
                Constant::Number(20.0),
                Constant::Number(30.0),
                Constant::Number(1.0),
                Constant::Number(99.0),
            ],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program("test_set_index_ref", &program).unwrap();

        let mut ctx = JITContext::default();
        let signal = unsafe { func(&mut ctx) };

        assert_eq!(signal, 0, "JIT execution should succeed");
        assert!(ctx.stack_ptr > 0, "Should have a result on stack");
        let result = ctx.stack[0];
        assert!(is_number(result), "Result should be a number");
        assert_eq!(
            unbox_number(result),
            99.0,
            "arr[1] should be 99.0 after SetIndexRef mutation"
        );
    }

    /// Regression test: jit_array_info FFI returns correct data_ptr and length.
    ///
    /// Directly tests the FFI function that replaced the unstable Vec memory
    /// layout assumption. Uses Rust's stable Vec API (as_ptr(), len()).
    #[test]
    fn test_jit_array_info_ffi() {
        use crate::ffi::array::jit_array_info;
        use crate::jit_array::JitArray;
        use crate::nan_boxing::{TAG_NULL, box_number};

        // Create a JitArray via heap_box (UnifiedArray self-boxing)
        let elements = vec![box_number(10.0), box_number(20.0), box_number(30.0)];
        let jit_arr = JitArray::from_vec(elements);
        let expected_len = jit_arr.len() as u64;
        let array_bits = jit_arr.heap_box();

        let info = jit_array_info(array_bits);
        assert_ne!(info.data_ptr, 0, "data_ptr should be non-null");
        assert_eq!(
            info.length, expected_len,
            "length should match JitArray::len()"
        );

        // Verify we can read elements through the returned pointer
        unsafe {
            let data = info.data_ptr as *const u64;
            assert_eq!(crate::nan_boxing::unbox_number(*data.add(0)), 10.0);
            assert_eq!(crate::nan_boxing::unbox_number(*data.add(1)), 20.0);
            assert_eq!(crate::nan_boxing::unbox_number(*data.add(2)), 30.0);
        }

        // Null/invalid inputs should return zeroes
        let null_info = jit_array_info(TAG_NULL);
        assert_eq!(null_info.data_ptr, 0);
        assert_eq!(null_info.length, 0);

        // Clean up
        unsafe {
            JitArray::heap_drop(array_bits);
        }
    }

    #[test]
    fn test_jit_width_aware_u8_add_wraps() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 250
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::simple(OpCode::AddInt), // 250 + 250 => 244 (u8 wrap)
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            ],
            constants: vec![Constant::Number(250.0)],
            top_level_locals_count: 1,
            top_level_local_storage_hints: vec![StorageHint::UInt8],
            ..Default::default()
        };
        assert_eq!(run_program_get_number("test_u8_wrap", program), 244.0);
    }

    #[test]
    fn test_jit_width_aware_u8_comparison_uses_unsigned_ordering() {
        use crate::nan_boxing::TAG_BOOL_FALSE;

        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 250
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::LtInt), // 250 < 1 => false (unsigned)
            ],
            constants: vec![Constant::Number(250.0), Constant::Number(1.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::UInt8, StorageHint::UInt8],
            ..Default::default()
        };
        assert_eq!(run_program_get_raw("test_u8_cmp", program), TAG_BOOL_FALSE);
    }

    #[test]
    fn test_jit_width_aware_i8_add_wraps_signed() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 120
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::simple(OpCode::AddInt), // 120 + 120 => -16 (i8 wrap)
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            ],
            constants: vec![Constant::Number(120.0)],
            top_level_locals_count: 1,
            top_level_local_storage_hints: vec![StorageHint::Int8],
            ..Default::default()
        };
        assert_eq!(run_program_get_number("test_i8_wrap", program), -16.0);
    }

    #[test]
    fn test_jit_width_aware_u16_add_wraps() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 65535
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::AddInt), // 65535 + 2 => 1 (u16 wrap)
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            ],
            constants: vec![Constant::Number(65535.0), Constant::Number(2.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::UInt16, StorageHint::UInt16],
            ..Default::default()
        };
        assert_eq!(run_program_get_number("test_u16_wrap", program), 1.0);
    }

    #[test]
    fn test_jit_width_aware_i16_signed_comparison() {
        use crate::nan_boxing::TAG_BOOL_TRUE;

        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // -1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::LtInt), // -1 < 1 => true (signed)
            ],
            constants: vec![Constant::Number(-1.0), Constant::Number(1.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::Int16, StorageHint::Int16],
            ..Default::default()
        };
        assert_eq!(run_program_get_raw("test_i16_cmp", program), TAG_BOOL_TRUE);
    }

    #[test]
    fn test_jit_width_aware_u16_unsigned_comparison() {
        use crate::nan_boxing::TAG_BOOL_TRUE;

        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 65535
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::GtInt), // 65535 > 1 => true (unsigned)
            ],
            constants: vec![Constant::Number(65535.0), Constant::Number(1.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::UInt16, StorageHint::UInt16],
            ..Default::default()
        };
        assert_eq!(run_program_get_raw("test_u16_cmp", program), TAG_BOOL_TRUE);
    }

    #[test]
    fn test_jit_width_aware_i32_add_wraps() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 2147483647
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::AddInt), // i32::MAX + 1 => i32::MIN
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            ],
            constants: vec![Constant::Number(2_147_483_647.0), Constant::Number(1.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::Int32, StorageHint::Int32],
            ..Default::default()
        };
        assert_eq!(
            run_program_get_number("test_i32_wrap", program),
            -2_147_483_648.0
        );
    }

    #[test]
    fn test_jit_width_aware_u32_add_wraps() {
        let program = BytecodeProgram {
            instructions: vec![
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 4294967295
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
                Instruction::simple(OpCode::AddInt), // u32::MAX + 1 => 0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            ],
            constants: vec![Constant::Number(4_294_967_295.0), Constant::Number(1.0)],
            top_level_locals_count: 2,
            top_level_local_storage_hints: vec![StorageHint::UInt32, StorageHint::UInt32],
            ..Default::default()
        };
        assert_eq!(run_program_get_number("test_u32_wrap", program), 0.0);
    }

    /// Regression test: nested loops with array access by computed index.
    ///
    /// Tests the pattern:
    ///   let arr = [10, 20, 30, 40, 50]
    ///   let sum = 0
    ///   for t in 0..3 {
    ///       for i in 0..5 {
    ///           sum = sum + arr[i]
    ///       }
    ///   }
    ///   // expect sum = 3 * (10+20+30+40+50) = 450
    ///
    /// This exercises nested loop compilation, LICM state management across
    /// loop boundaries, and integer unboxing with array indexing.
    #[test]
    fn test_jit_nested_loop_array_access() {
        // Constants: 0=10.0, 1=20.0, 2=30.0, 3=40.0, 4=50.0, 5=0.0, 6=3.0, 7=5.0, 8=1(int)
        let program = BytecodeProgram {
            instructions: vec![
                // Create array [10, 20, 30, 40, 50] → local 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 0: 10.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 1: 20.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),  // 2: 30.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),  // 3: 40.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),  // 4: 50.0
                Instruction::new(OpCode::NewArray, Some(Operand::Count(5))),   // 5
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 6: arr

                // sum = 0.0 → local 1
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 7: 0.0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 8: sum

                // Outer loop setup: t = 0, __end_t = 3
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 9: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 10
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 11: t = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(6))),  // 12: 3.0
                Instruction::simple(OpCode::NumberToInt),                       // 13
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(3))), // 14: __end_t = 3

                // Outer LoopStart
                Instruction::simple(OpCode::LoopStart),                        // 15
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 16: t
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 17: __end_t
                Instruction::simple(OpCode::LtInt),                            // 18: t < 3
                // JumpIfFalse to past outer LoopEnd (idx 48+1=49)
                // offset = 49 - (19+1) = 29
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(29))), // 19

                // Inner loop setup: i = 0, __end_i = 5
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 20: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 21
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 22: i = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(7))),  // 23: 5.0
                Instruction::simple(OpCode::NumberToInt),                       // 24
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(5))), // 25: __end_i = 5

                // Inner LoopStart
                Instruction::simple(OpCode::LoopStart),                        // 26
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 27: i
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(5))),  // 28: __end_i
                Instruction::simple(OpCode::LtInt),                            // 29: i < 5
                // JumpIfFalse to past inner LoopEnd (idx 42+1=43)
                // offset = 43 - (30+1) = 12
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(12))), // 30

                // Body: sum = sum + arr[i]
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 31: sum
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),  // 32: arr
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 33: i (index)
                Instruction::simple(OpCode::GetProp),                          // 34: arr[i]
                Instruction::simple(OpCode::Add),                              // 35: sum + arr[i]
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 36: sum = ...

                // Inner loop increment: i = i + 1
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 37: i
                Instruction::new(OpCode::PushConst, Some(Operand::Const(8))),  // 38: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 39: i + 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 40: i = ...
                // Jump back to inner LoopStart: target=26, offset = 26 - (41+1) = -16
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-16))),    // 41
                Instruction::simple(OpCode::LoopEnd),                          // 42

                // Outer loop increment: t = t + 1
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 43: t
                Instruction::new(OpCode::PushConst, Some(Operand::Const(8))),  // 44: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 45: t + 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 46: t = ...
                // Jump back to outer LoopStart: target=15, offset = 15 - (47+1) = -33
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-33))),    // 47
                Instruction::simple(OpCode::LoopEnd),                          // 48

                // Push result
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 49: sum
            ],
            constants: vec![
                Constant::Number(10.0), // 0
                Constant::Number(20.0), // 1
                Constant::Number(30.0), // 2
                Constant::Number(40.0), // 3
                Constant::Number(50.0), // 4
                Constant::Number(0.0),  // 5
                Constant::Number(3.0),  // 6
                Constant::Number(5.0),  // 7
                Constant::Int(1),       // 8
            ],
            top_level_locals_count: 6,
            top_level_local_storage_hints: vec![
                StorageHint::Unknown, // 0: arr (array)
                StorageHint::Float64, // 1: sum (number)
                StorageHint::Int64,   // 2: t (counter)
                StorageHint::Int64,   // 3: __end_t
                StorageHint::Int64,   // 4: i (counter)
                StorageHint::Int64,   // 5: __end_i
            ],
            ..Default::default()
        };

        let result = run_program_get_number("test_nested_loop_array", program);
        // Expected: 3 outer iterations * (10+20+30+40+50) = 3 * 150 = 450
        assert_eq!(result, 450.0, "nested loop array access should produce 450.0");
    }

    /// Regression test: nested loops with computed index (t * width + i).
    ///
    /// Tests the pattern:
    ///   let arr = Array.filled(6, 7.0)  // simulated as [7,7,7,7,7,7]
    ///   let sum = 0
    ///   for t in 0..2 {
    ///       for i in 0..3 {
    ///           let idx = t * 3 + i
    ///           sum = sum + arr[idx]
    ///       }
    ///   }
    ///   // expect sum = 6 * 7.0 = 42.0
    #[test]
    fn test_jit_nested_loop_computed_index() {
        // Constants: 0-5=7.0 (array), 6=0.0, 7=2.0, 8=3.0, 9=1(int), 10=3(int)
        let program = BytecodeProgram {
            instructions: vec![
                // Create array [7,7,7,7,7,7] → local 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 1
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 2
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 3
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 4
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 5
                Instruction::new(OpCode::NewArray, Some(Operand::Count(6))),   // 6
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 7: arr

                // sum = 0.0 → local 1
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 8: 0.0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 9: sum

                // Outer loop: t in 0..2, counter=local2, end=local3
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 10: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 11
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 12: t = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),  // 13: 2.0
                Instruction::simple(OpCode::NumberToInt),                       // 14
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(3))), // 15: __end_t = 2

                // Outer LoopStart
                Instruction::simple(OpCode::LoopStart),                        // 16
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 17: t
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 18: __end_t
                Instruction::simple(OpCode::LtInt),                            // 19: t < 2
                // JumpIfFalse → past outer LoopEnd at 55; offset = 56 - 21 = 35
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(35))), // 20

                // Inner loop: i in 0..3, counter=local4, end=local5
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 21: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 22
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 23: i = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),  // 24: 3.0
                Instruction::simple(OpCode::NumberToInt),                       // 25
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(5))), // 26: __end_i = 3

                // Inner LoopStart
                Instruction::simple(OpCode::LoopStart),                        // 27
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 28: i
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(5))),  // 29: __end_i
                Instruction::simple(OpCode::LtInt),                            // 30: i < 3
                // JumpIfFalse → past inner LoopEnd at 48; offset = 49 - 32 = 17
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(17))), // 31

                // Body: idx = t * 3 + i → local 6
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 32: t
                Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),  // 33: 3 (int)
                Instruction::simple(OpCode::MulInt),                           // 34: t * 3
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 35: i
                Instruction::simple(OpCode::AddInt),                           // 36: t*3 + i
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(6))), // 37: idx

                // sum = sum + arr[idx]
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 38: sum
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),  // 39: arr
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(6))),  // 40: idx
                Instruction::simple(OpCode::GetProp),                          // 41: arr[idx]
                Instruction::simple(OpCode::Add),                              // 42: sum + arr[idx]
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 43: sum = ...

                // Inner increment: i = i + 1
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 44: i
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 45: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 46: i + 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 47: i = ...
                // Jump → inner LoopStart at 27; offset = 27 - 49 = -22
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-22))),    // 48
                Instruction::simple(OpCode::LoopEnd),                          // 49

                // Outer increment: t = t + 1
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 50: t
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 51: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 52: t + 1
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 53: t = ...
                // Jump → outer LoopStart at 16; offset = 16 - 55 = -39
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-39))),    // 54
                Instruction::simple(OpCode::LoopEnd),                          // 55

                // Push result
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 56: sum
            ],
            constants: vec![
                Constant::Number(7.0),  // 0
                Constant::Number(0.0),  // 1
                Constant::Number(2.0),  // 2
                Constant::Number(3.0),  // 3
                Constant::Int(3),       // 4
                Constant::Int(1),       // 5
            ],
            top_level_locals_count: 7,
            top_level_local_storage_hints: vec![
                StorageHint::Unknown, // 0: arr
                StorageHint::Float64, // 1: sum
                StorageHint::Int64,   // 2: t
                StorageHint::Int64,   // 3: __end_t
                StorageHint::Int64,   // 4: i
                StorageHint::Int64,   // 5: __end_i
                StorageHint::Int64,   // 6: idx
            ],
            ..Default::default()
        };

        let result = run_program_get_number("test_nested_computed_idx", program);
        // Expected: 2 * 3 * 7.0 = 42.0
        assert_eq!(result, 42.0, "nested loop computed index should produce 42.0");
    }

    /// Stress test: many iterations of nested loops (500 x 5) with array access.
    ///
    /// Uses high outer loop count to stress-test LICM state management
    /// and array pointer stability across many iterations.
    /// Simulates the 500x127 pattern from load_xgb_model at reduced scale.
    #[test]
    fn test_jit_nested_loop_many_iterations() {
        // Constants: 0=10.0, 1=20.0, 2=30.0, 3=40.0, 4=50.0, 5=0.0, 6=500.0, 7=5.0, 8=1(int)
        let program = BytecodeProgram {
            instructions: vec![
                // Create array [10, 20, 30, 40, 50] → local 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 0: 10.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 1: 20.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),  // 2: 30.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),  // 3: 40.0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),  // 4: 50.0
                Instruction::new(OpCode::NewArray, Some(Operand::Count(5))),   // 5
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 6: arr

                // sum = 0.0 → local 1
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 7: 0.0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 8: sum

                // Outer loop: t in 0..500
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 9: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 10
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 11: t = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(6))),  // 12: 500.0
                Instruction::simple(OpCode::NumberToInt),                       // 13
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(3))), // 14: __end_t = 500

                Instruction::simple(OpCode::LoopStart),                        // 15
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 16
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 17
                Instruction::simple(OpCode::LtInt),                            // 18
                // JumpIfFalse → past outer LoopEnd at 48; offset = 49-20 = 29
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(29))), // 19

                // Inner loop: i in 0..5
                Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),  // 20: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 21
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 22: i = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(7))),  // 23: 5.0
                Instruction::simple(OpCode::NumberToInt),                       // 24
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(5))), // 25: __end_i = 5

                Instruction::simple(OpCode::LoopStart),                        // 26
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 27
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(5))),  // 28
                Instruction::simple(OpCode::LtInt),                            // 29
                // JumpIfFalse → past inner LoopEnd at 42; offset = 43-31 = 12
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(12))), // 30

                // Body: sum = sum + arr[i]
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 31: sum
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),  // 32: arr
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 33: i
                Instruction::simple(OpCode::GetProp),                          // 34: arr[i]
                Instruction::simple(OpCode::Add),                              // 35
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 36

                // Inner increment
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 37
                Instruction::new(OpCode::PushConst, Some(Operand::Const(8))),  // 38: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 39
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 40
                // Jump → inner LoopStart at 26; offset = 26-42 = -16
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-16))),    // 41
                Instruction::simple(OpCode::LoopEnd),                          // 42

                // Outer increment
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 43
                Instruction::new(OpCode::PushConst, Some(Operand::Const(8))),  // 44: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 45
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 46
                // Jump → outer LoopStart at 15; offset = 15-48 = -33
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-33))),    // 47
                Instruction::simple(OpCode::LoopEnd),                          // 48

                // Push result
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 49
            ],
            constants: vec![
                Constant::Number(10.0),  // 0
                Constant::Number(20.0),  // 1
                Constant::Number(30.0),  // 2
                Constant::Number(40.0),  // 3
                Constant::Number(50.0),  // 4
                Constant::Number(0.0),   // 5
                Constant::Number(500.0), // 6
                Constant::Number(5.0),   // 7
                Constant::Int(1),        // 8
            ],
            top_level_locals_count: 6,
            top_level_local_storage_hints: vec![
                StorageHint::Unknown, // 0: arr
                StorageHint::Float64, // 1: sum
                StorageHint::Int64,   // 2: t
                StorageHint::Int64,   // 3: __end_t
                StorageHint::Int64,   // 4: i
                StorageHint::Int64,   // 5: __end_i
            ],
            ..Default::default()
        };

        let result = run_program_get_number("test_nested_many_iter", program);
        // Expected: 500 * (10+20+30+40+50) = 500 * 150 = 75000
        assert_eq!(result, 75000.0, "many-iteration nested loop should produce 75000.0");
    }

    /// Test: nested loop accessing array via module binding (loads from ctx.locals memory).
    ///
    /// This tests the scenario where the array is a module-level variable,
    /// loaded via LoadModuleBinding (which reads from ctx.locals[] memory),
    /// rather than a local variable (which uses Cranelift Variables).
    #[test]
    fn test_jit_nested_loop_module_binding_array() {
        use crate::jit_array::JitArray;
        use crate::nan_boxing::box_number;

        // Create a pre-populated array of 10 elements
        let elements: Vec<u64> = (0..10).map(|i| box_number(i as f64)).collect();
        let arr = JitArray::from_vec(elements);
        let arr_bits = arr.heap_box();

        // Bytecode:
        //   module_binding 0 = arr (pre-loaded via ctx.locals[0])
        //   local 0 = sum = 0.0
        //   for t in 0..50:     (local 1 = t, local 2 = 50)
        //     for i in 0..10:   (local 3 = i, local 4 = 10)
        //       sum = sum + arr[i]
        //   push sum
        // Expected: 50 * sum(0..9) = 50 * 45 = 2250.0

        let program = BytecodeProgram {
            instructions: vec![
                // sum = 0.0 → local 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 0: 0.0
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 1: sum

                // Outer loop: t in 0..50
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 2: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 3
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 4: t = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),  // 5: 50.0
                Instruction::simple(OpCode::NumberToInt),                       // 6
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))), // 7: __end_t = 50

                Instruction::simple(OpCode::LoopStart),                        // 8
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 9
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(2))),  // 10
                Instruction::simple(OpCode::LtInt),                            // 11
                // JumpIfFalse → past outer LoopEnd at 41; offset = 42-13 = 29
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(29))), // 12

                // Inner loop: i in 0..10
                Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),  // 13: 0.0
                Instruction::simple(OpCode::NumberToInt),                       // 14
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(3))), // 15: i = 0
                Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),  // 16: 10.0
                Instruction::simple(OpCode::NumberToInt),                       // 17
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(4))), // 18: __end_i = 10

                Instruction::simple(OpCode::LoopStart),                        // 19
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 20
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(4))),  // 21
                Instruction::simple(OpCode::LtInt),                            // 22
                // JumpIfFalse → past inner LoopEnd at 35; offset = 36-24 = 12
                Instruction::new(OpCode::JumpIfFalse, Some(Operand::Offset(12))), // 23

                // Body: sum = sum + arr[i]
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),  // 24: sum
                // Load arr via module binding (reads from ctx.locals[0])
                Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))), // 25: arr
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 26: i
                Instruction::simple(OpCode::GetProp),                          // 27: arr[i]
                Instruction::simple(OpCode::Add),                              // 28
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))), // 29

                // Inner increment
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(3))),  // 30
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),  // 31: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 32
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(3))), // 33
                // Jump → inner LoopStart at 19; offset = 19-35 = -16
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-16))),    // 34
                Instruction::simple(OpCode::LoopEnd),                          // 35

                // Outer increment
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),  // 36
                Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),  // 37: 1 (int)
                Instruction::simple(OpCode::AddInt),                           // 38
                Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))), // 39
                // Jump → outer LoopStart at 8; offset = 8-41 = -33
                Instruction::new(OpCode::Jump, Some(Operand::Offset(-33))),    // 40
                Instruction::simple(OpCode::LoopEnd),                          // 41

                // Push result
                Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),  // 42
            ],
            constants: vec![
                Constant::Number(0.0),   // 0
                Constant::Number(50.0),  // 1
                Constant::Number(10.0),  // 2
                Constant::Int(1),        // 3
            ],
            top_level_locals_count: 5,
            top_level_local_storage_hints: vec![
                StorageHint::Float64, // 0: sum
                StorageHint::Int64,   // 1: t
                StorageHint::Int64,   // 2: __end_t
                StorageHint::Int64,   // 3: i
                StorageHint::Int64,   // 4: __end_i
            ],
            module_binding_names: vec!["arr".to_string()],
            module_binding_storage_hints: vec![StorageHint::Unknown],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(crate::context::JITConfig::default()).unwrap();
        let func = jit.compile_program("test_nested_mb_array", &program).unwrap();

        let mut ctx = JITContext::default();
        // Pre-load the array into module binding slot 0 (ctx.locals[0])
        ctx.locals[0] = arr_bits;

        let signal = unsafe { func(&mut ctx) };
        assert_eq!(signal, 0, "JIT execution should succeed");
        assert!(ctx.stack_ptr > 0, "Should have result on stack");

        let result = ctx.stack[0];
        assert!(is_number(result), "Result should be a number, got {result:#x}");
        let value = unbox_number(result);
        // Expected: 50 * sum(0..9) = 50 * 45 = 2250
        assert_eq!(value, 2250.0, "Module binding nested loop should produce 2250.0, got {value}");
    }
}
