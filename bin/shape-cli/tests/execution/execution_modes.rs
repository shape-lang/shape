//! Execution Mode Tests
//!
//! Tests that verify the Shape engine executes correctly in different modes.

use crate::common::init_runtime;
use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use std::time::Instant;

struct Strategy {
    name: &'static str,
    code: &'static str,
}

const SIMPLE_STRATEGY: Strategy = Strategy {
    name: "Simple Arithmetic",
    code: "let a = 10; let b = 20; let c = a + b * 2; c",
};

const MEDIUM_STRATEGY: Strategy = Strategy {
    name: "Loop with Conditionals",
    code: "let mut total = 0; for i in range(100) { if i % 2 == 0 { total = total + i } }; total",
};

const COMPLEX_STRATEGY: Strategy = Strategy {
    name: "Recursive Fibonacci",
    code: "function fibonacci(n) { if n <= 1 { return n } else { return fibonacci(n-1) + fibonacci(n-2) } }; fibonacci(15)",
};

#[test]
fn test_engine_executes_simple() {
    init_runtime();
    let mut engine = ShapeEngine::new().expect("Failed to create engine");
    let mut executor = BytecodeExecutor::new();

    let result = engine.execute(&mut executor, SIMPLE_STRATEGY.code);
    assert!(result.is_ok(), "Simple strategy failed: {:?}", result);
}

#[test]
fn test_engine_executes_medium() {
    init_runtime();
    let mut engine = ShapeEngine::new().expect("Failed to create engine");
    let mut executor = BytecodeExecutor::new();

    let result = engine.execute(&mut executor, MEDIUM_STRATEGY.code);
    assert!(result.is_ok(), "Medium strategy failed: {:?}", result);
}

#[test]
fn test_engine_executes_complex() {
    init_runtime();
    let mut engine = ShapeEngine::new().expect("Failed to create engine");
    let mut executor = BytecodeExecutor::new();

    let result = engine.execute(&mut executor, COMPLEX_STRATEGY.code);
    assert!(result.is_ok(), "Complex strategy failed: {:?}", result);
}

#[test]
// Benchmark: runs 100 iterations of each strategy
fn execution_benchmark() {
    init_runtime();

    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║     Shape Execution Benchmark                                  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    for strategy in [&SIMPLE_STRATEGY, &MEDIUM_STRATEGY, &COMPLEX_STRATEGY] {
        let mut engine = ShapeEngine::new().expect("Failed to create engine");
        let mut executor = BytecodeExecutor::new();

        // Warm up
        let _ = engine.execute(&mut executor, strategy.code);

        // Benchmark
        let iterations = 100;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = engine.execute(&mut executor, strategy.code);
        }
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        println!(
            "{}: {:.3}ms avg ({} iterations)",
            strategy.name, avg_ms, iterations
        );
    }
}
