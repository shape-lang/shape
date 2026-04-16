//! Differential testing for trusted opcodes.
//!
//! Compiles the same program twice — once with trusted opcodes (normal
//! compilation) and once with all trusted opcodes downgraded to their
//! guarded counterparts — then verifies both produce identical results.
//!
//! This catches any divergence between the trusted (guard-free) and
//! guarded (runtime-checked) execution paths.

use super::*;
use crate::bytecode::{BytecodeProgram, OpCode};
use shape_value::{VMError, ValueWord, ValueWordExt};

/// Compile and run a Shape program normally (trusted opcodes may be emitted).
fn run_with_trusted(source: &str) -> Result<ValueWord, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Post-process bytecode to downgrade all trusted opcodes to their guarded
/// counterparts, then run.
fn run_guarded_only(source: &str) -> Result<ValueWord, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.set_source(source);
    let mut bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    downgrade_trusted_opcodes(&mut bytecode);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Replace all *Trusted opcodes in the program with their guarded equivalents.
fn downgrade_trusted_opcodes(program: &mut BytecodeProgram) {
    for instr in program.instructions.iter_mut() {
        if let Some(guarded) = instr.opcode.guarded_variant() {
            instr.opcode = guarded;
        }
    }
}

/// Assert that trusted and guarded execution produce the same result.
fn assert_same(source: &str) {
    let trusted = run_with_trusted(source);
    let guarded = run_guarded_only(source);
    match (&trusted, &guarded) {
        (Ok(t), Ok(g)) => {
            // Compare as i64 first, then f64, then string representation
            if let (Some(ti), Some(gi)) = (t.as_i64(), g.as_i64()) {
                assert_eq!(ti, gi, "Int mismatch for: {}", source);
            } else if let (Some(tf), Some(gf)) = (t.as_f64(), g.as_f64()) {
                assert!(
                    (tf - gf).abs() < 1e-10 || (tf.is_nan() && gf.is_nan()),
                    "Float mismatch for: {} (trusted={}, guarded={})",
                    source,
                    tf,
                    gf
                );
            } else if let (Some(tb), Some(gb)) = (t.as_bool(), g.as_bool()) {
                assert_eq!(tb, gb, "Bool mismatch for: {}", source);
            } else {
                // Fallback: both should have the same tag structure
                assert_eq!(
                    format!("{}", t),
                    format!("{}", g),
                    "Display mismatch for: {}",
                    source
                );
            }
        }
        (Err(te), Err(ge)) => {
            // Both errored — acceptable (e.g., division by zero)
            assert_eq!(
                std::mem::discriminant(te),
                std::mem::discriminant(ge),
                "Error type mismatch for: {} (trusted={:?}, guarded={:?})",
                source,
                te,
                ge
            );
        }
        _ => {
            panic!(
                "Result mismatch for: {} — trusted={:?}, guarded={:?}",
                source, trusted, guarded
            );
        }
    }
}

// ── Differential: int arithmetic ────────────────────────────────────

#[test]
fn differential_int_addition() {
    let programs = vec![
        "1 + 2",
        "0 + 0",
        "-1 + 1",
        "100 + 200 + 300",
        "let x = 10\nlet y = 20\nx + y",
    ];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_int_subtraction() {
    let programs = vec![
        "5 - 3",
        "0 - 0",
        "3 - 5",
        "100 - 200",
        "let x = 50\nlet y = 30\nx - y",
    ];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_int_multiplication() {
    let programs = vec![
        "3 * 4",
        "0 * 100",
        "-3 * 7",
        "100 * 200 + 50",
        "let x = 10\nlet y = 20\nx * y + 3",
    ];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_int_division() {
    let programs = vec![
        "10 / 2",
        "7 / 3",
        "100 / 10",
        "-10 / 3",
        "let x = 100\nlet y = 4\nx / y",
    ];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_int_div_by_zero() {
    // Both paths should produce the same error
    assert_same("let x = 10\nlet y = 0\nx / y");
}

// ── Differential: float arithmetic ──────────────────────────────────

#[test]
fn differential_float_addition() {
    let programs = vec!["1.5 + 2.5", "0.0 + 0.0", "-1.5 + 1.5", "3.14 + 2.72"];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_float_subtraction() {
    let programs = vec!["5.5 - 3.3", "0.0 - 0.0", "1.0 - 2.0"];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_float_multiplication() {
    let programs = vec!["2.5 * 4.0", "0.0 * 100.0", "-3.0 * 7.0"];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_float_division() {
    let programs = vec!["10.0 / 3.0", "1.0 / 7.0", "100.0 / 0.5"];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_float_div_by_zero() {
    assert_same("let x = 10.0\nlet y = 0.0\nx / y");
}

// ── Differential: mixed arithmetic ──────────────────────────────────

#[test]
fn differential_mixed_int_float() {
    // These may trigger int->number coercion
    let programs = vec![
        "let x = 10\nlet y = 3.5\nx + y",
        "let x = 10\nlet y = 3.5\nx * y",
        "let x = 10\nlet y = 3.5\nx - y",
    ];
    for src in programs {
        assert_same(src);
    }
}

// ── Differential: comparison operations ─────────────────────────────

#[test]
fn differential_int_comparisons() {
    let programs = vec![
        "3 > 2", "2 > 3", "3 >= 3", "3 >= 4", "2 < 3", "3 < 2", "3 <= 3", "3 <= 2",
    ];
    for src in programs {
        assert_same(src);
    }
}

#[test]
fn differential_float_comparisons() {
    let programs = vec![
        "3.0 > 2.0",
        "2.0 > 3.0",
        "3.0 >= 3.0",
        "2.0 < 3.0",
        "3.0 <= 3.0",
    ];
    for src in programs {
        assert_same(src);
    }
}

// ── Differential: complex expressions ───────────────────────────────

#[test]
fn differential_complex_expressions() {
    let programs = vec![
        "let x = 10\nlet y = 20\nx + y * 3",
        "let a = 5\nlet b = 3\nlet c = 2\na * b + c",
        "let a = 100\nlet b = 7\na / b * b + a % b",
    ];
    for src in programs {
        assert_same(src);
    }
}

// ── Differential: loop with trusted arithmetic ──────────────────────

#[test]
fn differential_loop_sum() {
    let source = r#"
        let sum = 0
        let i = 0
        while i < 1000 {
            sum = sum + i
            i = i + 1
        }
        sum
    "#;
    assert_same(source);
}

#[test]
fn differential_loop_product() {
    // Factorial of 10
    let source = r#"
        let prod = 1
        let i = 1
        while i <= 10 {
            prod = prod * i
            i = i + 1
        }
        prod
    "#;
    assert_same(source);
}

// ── Differential: function with trusted opcodes ─────────────────────

#[test]
fn differential_function_arithmetic() {
    let source = r#"
        fn add(a: int, b: int) -> int {
            a + b
        }
        fn mul(a: int, b: int) -> int {
            a * b
        }
        add(3, 4) + mul(5, 6)
    "#;
    assert_same(source);
}

#[test]
fn differential_recursive_function() {
    let source = r#"
        fn fib(n: int) -> int {
            if n <= 1 {
                n
            } else {
                fib(n - 1) + fib(n - 2)
            }
        }
        fib(15)
    "#;
    assert_same(source);
}
