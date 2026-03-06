//! VM bytecode execution performance benchmarks
//!
//! Measures the performance of the Shape virtual machine executing compiled bytecode

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use shape_core::ast::Program;
use shape_core::parser::parse_program;
use shape_core::vm::bytecode::{Constant, Operand};
use shape_core::vm::{
    BytecodeCompiler, BytecodeProgram, Instruction, OpCode, VMConfig, VirtualMachine,
};

fn execute_program(bytecode: &BytecodeProgram) {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode.clone());
    vm.execute(None).unwrap();
}

fn compile_program(program: &Program) -> BytecodeProgram {
    BytecodeCompiler::new()
        .compile(program)
        .expect("program should compile for benchmarks")
}

/// Compile and execute simple arithmetic expressions
fn benchmark_arithmetic_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_arithmetic");

    // Simple arithmetic
    let expressions = vec![
        ("simple_add", "1 + 2"),
        ("simple_multiply", "5 * 10"),
        ("complex_arithmetic", "(10 + 5) * 3 - 8 / 2"),
        (
            "nested_arithmetic",
            "((2 + 3) * (4 + 5)) / ((6 + 7) - (8 - 9))",
        ),
    ];

    for (name, expr_str) in expressions {
        let program = format!("let result = {};", expr_str);
        let ast = parse_program(&program).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_function(name, |b| {
            b.iter(|| {
                execute_program(black_box(&bytecode));
            });
        });
    }

    // Benchmark arithmetic with many operations
    for &num_ops in &[10, 50, 100, 500] {
        let expr = (0..num_ops)
            .map(|i| format!("{}", i))
            .collect::<Vec<_>>()
            .join(" + ");

        let program = format!("let result = {};", expr);
        let ast = parse_program(&program).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_with_input(
            BenchmarkId::new("addition_chain", num_ops),
            &bytecode,
            |b, bytecode| {
                b.iter(|| {
                    execute_program(black_box(bytecode));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark variable operations and scoping
fn benchmark_variable_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_variables");

    // Variable declaration and assignment
    let var_program = r#"
        let x = 10;
        let y = 20;
        let z = x + y;
        var w = z * 2;
        w = w + 10;
        const pi = 3.14159;
        let result = w * pi;
    "#;

    let ast = parse_program(var_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("variable_operations", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Nested scopes
    let scope_program = r#"
        let outer = 100;
        {
            let inner = 50;
            let sum = outer + inner;
            {
                let deep = 25;
                let total = sum + deep;
            }
        }
        let final = outer * 2;
    "#;

    let ast = parse_program(scope_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("nested_scopes", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Many variables
    for &num_vars in &[10, 50, 100, 500] {
        let mut program = String::new();
        for i in 0..num_vars {
            program.push_str(&format!("let var_{} = {};\n", i, i));
        }
        program.push_str("let sum = ");
        program.push_str(
            &(0..num_vars)
                .map(|i| format!("var_{}", i))
                .collect::<Vec<_>>()
                .join(" + "),
        );
        program.push_str(";");

        let ast = parse_program(&program).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_with_input(
            BenchmarkId::new("many_variables", num_vars),
            &bytecode,
            |b, bytecode| {
                b.iter(|| {
                    execute_program(black_box(bytecode));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark function calls
fn benchmark_function_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_functions");

    // Simple function
    let simple_func = r#"
        function add(a, b) {
            return a + b;
        }
        
        let result = add(10, 20);
    "#;

    let ast = parse_program(simple_func).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("simple_function_call", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Recursive function (factorial)
    let recursive_func = r#"
        function factorial(n) {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }
        
        let result = factorial(10);
    "#;

    let ast = parse_program(recursive_func).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("recursive_function", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Function with closure
    let closure_func = r#"
        function make_counter() {
            let count = 0;
            return function() {
                count = count + 1;
                return count;
            };
        }
        
        let counter = make_counter();
        let a = counter();
        let b = counter();
        let c = counter();
    "#;

    let ast = parse_program(closure_func).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("closure_function", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Many function calls
    for &num_calls in &[10, 50, 100] {
        let mut program = r#"
            function compute(x) {
                return x * 2 + 1;
            }
        "#
        .to_string();

        for i in 0..num_calls {
            program.push_str(&format!("let result_{} = compute({});\n", i, i));
        }

        let ast = parse_program(&program).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_with_input(
            BenchmarkId::new("many_function_calls", num_calls),
            &bytecode,
            |b, bytecode| {
                b.iter(|| {
                    execute_program(black_box(bytecode));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark control flow operations
fn benchmark_control_flow(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_control_flow");

    // If-else branches
    let if_else_program = r#"
        let x = 10;
        let result;
        if x > 5 {
            result = x * 2;
        } else {
            result = x / 2;
        }
    "#;

    let ast = parse_program(if_else_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("if_else", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // While loop
    let while_program = r#"
        let i = 0;
        let sum = 0;
        while i < 100 {
            sum = sum + i;
            i = i + 1;
        }
    "#;

    let ast = parse_program(while_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("while_loop", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // For loop
    let for_program = r#"
        let sum = 0;
        for i in range(100) {
            sum = sum + i;
        }
    "#;

    let ast = parse_program(for_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("for_loop", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Nested loops
    for &size in &[10, 20, 50] {
        let nested_program = format!(
            r#"
            let sum = 0;
            for i in range({}) {{
                for j in range({}) {{
                    sum = sum + i * j;
                }}
            }}
        "#,
            size, size
        );

        let ast = parse_program(&nested_program).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_with_input(
            BenchmarkId::new("nested_loops", size),
            &bytecode,
            |b, bytecode| {
                b.iter(|| {
                    execute_program(black_box(bytecode));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark array and object operations
fn benchmark_collections(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_collections");

    // Array operations
    let array_program = r#"
        let arr = [1, 2, 3, 4, 5];
        let sum = 0;
        for val in arr {
            sum = sum + val;
        }
        arr.push(6);
        let last = arr.pop();
        let sliced = arr.slice(1, 3);
    "#;

    let ast = parse_program(array_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("array_operations", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Object operations
    let object_program = r#"
        let obj = {
            name: "test",
            value: 42,
            nested: {
                x: 10,
                y: 20
            }
        };
        
        let name = obj.name;
        let x = obj.nested.x;
        obj.new_field = 100;
        obj["dynamic_key"] = 200;
    "#;

    let ast = parse_program(object_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("object_operations", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Large arrays
    for &size in &[100, 500, 1000] {
        let array_creation = format!(
            "let arr = [{}];",
            (0..size)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let ast = parse_program(&array_creation).unwrap();
        let bytecode = compile_program(&ast);

        group.bench_with_input(
            BenchmarkId::new("array_creation", size),
            &bytecode,
            |b, bytecode| {
                b.iter(|| {
                    execute_program(black_box(bytecode));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark pattern matching execution
fn benchmark_pattern_matching_vm(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_pattern_matching");

    // Simple pattern
    let simple_pattern = r#"
        pattern hammer {
            body = abs(close - open);
            range = high - low;
            body < range * 0.3
        }
        
        function check_pattern(row) {
            if row matches hammer {
                return true;
            }
            return false;
        }

        // Simulate checking multiple rows
        let matches = 0;
        for i in range(100) {
            let row = {
                open: 100 + i * 0.1,
                high: 101 + i * 0.1,
                low: 99 + i * 0.1,
                close: 100.5 + i * 0.1
            };

            if check_pattern(row) {
                matches = matches + 1;
            }
        }
    "#;

    let ast = parse_program(simple_pattern).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("pattern_matching", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Complex pattern with fuzzy matching
    let fuzzy_pattern = r#"
        pattern doji ~0.05 {
            body = abs(close - open);
            range = high - low;
            body ~< range * 0.1
        }
        
        pattern engulfing {
            data[-1].close < data[-1].open and
            data[0].open < data[-1].close and
            data[0].close > data[-1].open
        }

        let doji_count = 0;
        let engulfing_count = 0;

        for i in range(1, 100) {
            if data[i] matches doji {
                doji_count = doji_count + 1;
            }
            if data[i] matches engulfing {
                engulfing_count = engulfing_count + 1;
            }
        }
    "#;

    let ast = parse_program(fuzzy_pattern).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("fuzzy_pattern_matching", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    group.finish();
}

/// Benchmark indicator calculations
fn benchmark_indicator_calculations(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_indicators");

    // SMA calculation
    let sma_program = r#"
        function sma(data, period) {
            let sum = 0;
            let count = 0;
            
            for i in range(data.length) {
                if i >= data.length - period {
                    sum = sum + data[i];
                    count = count + 1;
                }
            }
            
            return sum / count;
        }
        
        let prices = [];
        for i in range(100) {
            prices.push(100 + sin(i * 0.1) * 10);
        }
        
        let sma20 = sma(prices, 20);
        let sma50 = sma(prices, 50);
    "#;

    let ast = parse_program(sma_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("sma_calculation", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // RSI calculation
    let rsi_program = r#"
        function rsi(data, period) {
            let gains = [];
            let losses = [];
            
            for i in range(1, data.length) {
                let change = data[i] - data[i-1];
                if change > 0 {
                    gains.push(change);
                    losses.push(0);
                } else {
                    gains.push(0);
                    losses.push(-change);
                }
            }
            
            let avg_gain = 0;
            let avg_loss = 0;
            
            // Initial averages
            for i in range(period) {
                avg_gain = avg_gain + gains[i];
                avg_loss = avg_loss + losses[i];
            }
            avg_gain = avg_gain / period;
            avg_loss = avg_loss / period;
            
            // Calculate RSI
            if avg_loss == 0 {
                return 100;
            }
            
            let rs = avg_gain / avg_loss;
            return 100 - (100 / (1 + rs));
        }
        
        let prices = [];
        for i in range(100) {
            prices.push(100 + sin(i * 0.1) * 10 + cos(i * 0.05) * 5);
        }
        
        let rsi14 = rsi(prices, 14);
    "#;

    let ast = parse_program(rsi_program).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("rsi_calculation", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    group.finish();
}

/// Benchmark VM instruction dispatch overhead
fn benchmark_instruction_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_dispatch");

    // Minimal instructions
    let mut minimal_program = BytecodeProgram::new();
    let const_idx = minimal_program.add_constant(Constant::Number(42.0));
    minimal_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(const_idx)),
    ));
    minimal_program.emit(Instruction::simple(OpCode::Pop));

    group.bench_function("minimal_dispatch", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                execute_program(black_box(&minimal_program));
            }
        });
    });

    // Mixed instruction types
    let mut mixed_program = BytecodeProgram::new();
    let c10 = mixed_program.add_constant(Constant::Number(10.0));
    let c20 = mixed_program.add_constant(Constant::Number(20.0));
    let c5 = mixed_program.add_constant(Constant::Number(5.0));
    let c_str = mixed_program.add_constant(Constant::String("test".to_string()));
    let c_true = mixed_program.add_constant(Constant::Bool(true));

    mixed_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c10)),
    ));
    mixed_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c20)),
    ));
    mixed_program.emit(Instruction::simple(OpCode::Add));
    mixed_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c5)),
    ));
    mixed_program.emit(Instruction::simple(OpCode::Mul));
    mixed_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_str)),
    ));
    mixed_program.emit(Instruction::simple(OpCode::Pop));
    mixed_program.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(c_true)),
    ));
    mixed_program.emit(Instruction::simple(OpCode::Not));

    group.bench_function("mixed_instructions", |b| {
        b.iter(|| {
            for _ in 0..100 {
                execute_program(black_box(&mixed_program));
            }
        });
    });

    group.finish();
}

/// Benchmark memory allocation patterns
fn benchmark_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_memory");

    // String concatenation
    let string_concat = r#"
        let result = "";
        for i in range(100) {
            result = result + "x";
        }
    "#;

    let ast = parse_program(string_concat).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("string_concatenation", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    // Object creation in loop
    let object_creation = r#"
        let objects = [];
        for i in range(100) {
            objects.push({
                id: i,
                value: i * 2,
                data: [i, i+1, i+2]
            });
        }
    "#;

    let ast = parse_program(object_creation).unwrap();
    let bytecode = compile_program(&ast);

    group.bench_function("object_allocation", |b| {
        b.iter(|| {
            execute_program(black_box(&bytecode));
        });
    });

    group.finish();
}

// Helper function to simulate sin for deterministic benchmarks
fn sin(x: f64) -> f64 {
    // Simple approximation for benchmark determinism
    let x = x % (2.0 * std::f64::consts::PI);
    x - (x * x * x) / 6.0 + (x * x * x * x * x) / 120.0
}

fn cos(x: f64) -> f64 {
    sin(x + std::f64::consts::FRAC_PI_2)
}

criterion_group!(
    benches,
    benchmark_arithmetic_operations,
    benchmark_variable_operations,
    benchmark_function_calls,
    benchmark_control_flow,
    benchmark_collections,
    benchmark_pattern_matching_vm,
    benchmark_indicator_calculations,
    benchmark_instruction_dispatch,
    benchmark_memory_patterns
);
criterion_main!(benches);
