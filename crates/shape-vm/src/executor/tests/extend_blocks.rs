//! Extend Block UFCS Execution Tests
//!
//! These tests verify the complete extend block execution including:
//! - Basic method extension on built-in types
//! - Generic type extension (Vec<T>)
//! - Multiple methods in one extend block
//! - `self` binding correctness
//! - UFCS (Uniform Function Call Syntax) dispatch

use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::{VMConfig, VMError};
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

/// Extract a numeric value from ValueWord, accepting both Number and Int variants
fn as_f64(v: &ValueWord) -> Option<f64> {
    v.as_number_coerce()
}

/// Helper to compile and execute a Shape program
fn compile_and_execute(source: &str) -> Result<ValueWord, VMError> {
    // Parse the program
    let program = parse_program(source).map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;

    // Compile to bytecode
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;

    // Execute
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

#[test]
fn test_extend_number_basic() {
    // Test: extend Number { method double() { self * 2 } } → (5).double() = 10
    let source = r#"
        extend Number {
            method double() {
                return self * 2
            }
        }

        (5).double()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Basic Number extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(10.0),
        "5.double() should return 10, got {:?}",
        val
    );
}

#[test]
fn test_extend_number_with_param() {
    // Test that extend methods can take parameters
    let source = r#"
        extend Number {
            method add(n) {
                return self + n
            }
        }

        (5).add(3)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Number extension with param should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(8.0),
        "5.add(3) should return 8, got {:?}",
        val
    );
}

#[test]
fn test_extend_number_multiple_methods() {
    // Test: Extend with multiple methods, verify all callable via UFCS
    let source = r#"
        extend Number {
            method double() {
                return self * 2
            }

            method triple() {
                return self * 3
            }

            method square() {
                return self * self
            }
        }

        let x = 5;
        let doubled = x.double();
        let tripled = x.triple();
        let squared = x.square();

        [doubled, tripled, squared]
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Multiple methods should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    let arr = binding.as_array().expect("Expected Vec");
    assert_eq!(arr.len(), 3, "Should have 3 results");

    assert_eq!(
        as_f64(&arr[0]),
        Some(10.0),
        "double() should return 10, got {:?}",
        arr[0]
    );
    assert_eq!(
        as_f64(&arr[1]),
        Some(15.0),
        "triple() should return 15, got {:?}",
        arr[1]
    );
    assert_eq!(
        as_f64(&arr[2]),
        Some(25.0),
        "square() should return 25, got {:?}",
        arr[2]
    );
}

#[test]
fn test_extend_string_basic() {
    // Test extending String type
    let source = r#"
        extend String {
            method repeat(times) {
                var result = "";
                var i = 0;
                while (i < times) {
                    result = result + self;
                    i = i + 1;
                }
                return result
            }
        }

        "hi".repeat(3)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "String extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    let s = val.as_arc_string().expect("Expected String");
    assert_eq!(s.as_ref(), "hihihi", "Should repeat 3 times");
}

#[test]
fn test_extend_array_basic() {
    // Test extending Vec type
    let source = r#"
        extend Vec {
            method sum() {
                var total = 0;
                var i = 0;
                while (i < self.length()) {
                    total = total + self[i];
                    i = i + 1;
                }
                return total
            }
        }

        [1, 2, 3, 4, 5].sum()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Vec extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(15.0),
        "Sum of [1,2,3,4,5] should be 15, got {:?}",
        val
    );
}

#[test]
fn test_extend_array_generic() {
    // Test: extend Vec<T> { method ... } → verify generic types
    // Note: This tests that Vec methods work regardless of element type
    let source = r#"
        extend Vec {
            method first() {
                if (self.length() > 0) {
                    return self[0]
                }
                return None
            }

            method last() {
                let len = self.length();
                if (len > 0) {
                    return self[len - 1]
                }
                return None
            }
        }

        // Test with number array
        let nums = [10, 20, 30];
        let num_first = nums.first();
        let num_last = nums.last();

        // Test with string array
        let strings = ["a", "b", "c"];
        let str_first = strings.first();
        let str_last = strings.last();

        [num_first, num_last, str_first, str_last]
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Generic Vec extension should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    let arr = binding.as_array().expect("Expected Vec");
    assert_eq!(arr.len(), 4, "Should have 4 results");

    assert_eq!(
        as_f64(&arr[0]),
        Some(10.0),
        "First number should be 10, got {:?}",
        arr[0]
    );
    assert_eq!(
        as_f64(&arr[1]),
        Some(30.0),
        "Last number should be 30, got {:?}",
        arr[1]
    );

    let s2 = arr[2].as_arc_string().expect("Expected String");
    assert_eq!(s2.as_ref(), "a", "First string should be 'a'");

    let s3 = arr[3].as_arc_string().expect("Expected String");
    assert_eq!(s3.as_ref(), "c", "Last string should be 'c'");
}

#[test]
fn test_extend_this_binding_in_nested_context() {
    // Test: `self` binding correctness in various contexts
    let source = r#"
        extend Number {
            method add_and_multiply(a, b) {
                // `self` should refer to the number, not get confused
                // even when used in nested expressions
                let sum = self + a;
                let product = sum * b;
                return product
            }
        }

        (10).add_and_multiply(5, 2)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Complex self binding should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(30.0),
        "Should get (10 + 5) * 2 = 30, got {:?}",
        val
    );
}

#[test]
fn test_extend_this_in_closure() {
    // Test that `self` is correctly bound when methods use loops referencing self
    let source = r#"
        extend Vec {
            method double_all() {
                var result = [];
                var i = 0;
                while (i < self.length()) {
                    result = result.push(self[i] * 2);
                    i = i + 1;
                }
                return result
            }
        }

        [1, 2, 3].double_all()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "This in closure context should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    let arr = binding.as_array().expect("Expected Vec");
    assert_eq!(arr.len(), 3, "Should have 3 elements");

    assert_eq!(as_f64(&arr[0]), Some(2.0), "Expected 2, got {:?}", arr[0]);
    assert_eq!(as_f64(&arr[1]), Some(4.0), "Expected 4, got {:?}", arr[1]);
    assert_eq!(as_f64(&arr[2]), Some(6.0), "Expected 6, got {:?}", arr[2]);
}

#[test]
fn test_extend_chained_method_calls() {
    // Test that extended methods can be chained
    let source = r#"
        extend Number {
            method add(n) {
                return self + n
            }

            method multiply(n) {
                return self * n
            }
        }

        (5).add(3).multiply(2)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Chained method calls should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(16.0),
        "Should get (5 + 3) * 2 = 16, got {:?}",
        val
    );
}

#[test]
fn test_extend_method_with_default_param() {
    // Test that extended methods can use default parameters
    let source = r#"
        extend Number {
            method power(exponent = 2) {
                var result = 1;
                var i = 0;
                while (i < exponent) {
                    result = result * self;
                    i = i + 1;
                }
                return result
            }
        }

        // Call with explicit param
        let with_3 = (5).power(3);

        // Call with default param (should be 2)
        let with_default = (5).power();

        [with_3, with_default]
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Methods with default params should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    let arr = binding.as_array().expect("Expected Vec");
    assert_eq!(arr.len(), 2, "Should have 2 results");

    assert_eq!(
        as_f64(&arr[0]),
        Some(125.0),
        "5^3 should be 125, got {:?}",
        arr[0]
    );
    assert_eq!(
        as_f64(&arr[1]),
        Some(25.0),
        "5^2 should be 25, got {:?}",
        arr[1]
    );
}

#[test]
fn test_extend_multiple_types() {
    // Test that we can extend multiple types in the same program
    let source = r#"
        extend Number {
            method negate() {
                return -self
            }
        }

        extend String {
            method upper() {
                // Note: Actual toUpperCase() might not be implemented yet
                // This is a simple test placeholder
                return self
            }
        }

        extend Vec {
            method count() {
                return self.length()
            }
        }

        let num = (42).negate();
        let str = "hello".upper();
        let cnt = [1, 2, 3].count();

        [num, str, cnt]
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Multiple type extensions should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    let arr = binding.as_array().expect("Expected Vec");
    assert_eq!(arr.len(), 3, "Should have 3 results");

    assert_eq!(
        as_f64(&arr[0]),
        Some(-42.0),
        "Should negate to -42, got {:?}",
        arr[0]
    );

    let s = arr[1].as_arc_string().expect("Expected String");
    assert_eq!(s.as_ref(), "hello");

    assert_eq!(
        as_f64(&arr[2]),
        Some(3.0),
        "Should count to 3, got {:?}",
        arr[2]
    );
}
