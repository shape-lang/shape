//! End-to-end integration tests for typed arrays.
//!
//! These tests compile Shape source code and execute it, verifying that
//! typed array operations produce correct results through the full pipeline:
//! parse → compile → bytecode → VM execution.
//!
//! Currently these exercise the v1 typed array path (NewTypedArray) since the
//! bytecode compiler does not yet emit v2 typed array opcodes. The v2 path
//! (NewTypedArrayF64 etc.) is tested via direct bytecode tests in v2_opcode_tests.

use crate::test_utils::eval;

// ===== Array<number> (f64) =====

#[test]
fn test_typed_array_f64_literal_sum() {
    let result = eval(
        "
        let arr = [1.0, 2.0, 3.0, 4.0, 5.0]
        let mut sum = 0.0
        for x in arr {
            sum = sum + x
        }
        sum
    ",
    );
    let val = result.to_number().expect("expected number");
    assert!((val - 15.0).abs() < 1e-10);
}

#[test]
fn test_typed_array_f64_index_access() {
    let result = eval(
        "
        let arr = [10.5, 20.5, 30.5]
        arr[1]
    ",
    );
    let val = result.to_number().expect("expected number");
    assert!((val - 20.5).abs() < 1e-10);
}

#[test]
fn test_typed_array_f64_len() {
    let result = eval(
        "
        let arr = [1.0, 2.0, 3.0]
        arr.len()
    ",
    );
    assert_eq!(result.as_i64(), Some(3));
}

// ===== Array<int> (i64) =====

#[test]
fn test_typed_array_int_literal_sum() {
    let result = eval(
        "
        let arr = [10, 20, 30]
        let mut sum = 0
        for x in arr {
            sum = sum + x
        }
        sum
    ",
    );
    assert_eq!(result.as_i64(), Some(60));
}

#[test]
fn test_typed_array_int_index_access() {
    let result = eval(
        "
        let arr = [10, 20, 30]
        arr[1]
    ",
    );
    assert_eq!(result.as_i64(), Some(20));
}

#[test]
fn test_typed_array_int_len() {
    let result = eval(
        "
        let arr = [1, 2, 3, 4, 5]
        arr.len()
    ",
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_typed_array_int_first_last() {
    let first = eval(
        "
        let arr = [10, 20, 30]
        arr.first()
    ",
    );
    assert_eq!(first.as_i64(), Some(10));

    let last = eval(
        "
        let arr = [10, 20, 30]
        arr.last()
    ",
    );
    assert_eq!(last.as_i64(), Some(30));
}

// ===== Array mutation =====

#[test]
fn test_typed_array_push_and_len() {
    let result = eval(
        "
        let mut arr = [1, 2]
        arr.push(3)
        arr.push(4)
        arr.len()
    ",
    );
    assert_eq!(result.as_i64(), Some(4));
}

// ===== Array iteration =====

#[test]
fn test_typed_array_for_in_accumulate() {
    let result = eval(
        "
        let arr = [2, 4, 6, 8]
        let mut product = 1
        for x in arr {
            product = product * x
        }
        product
    ",
    );
    assert_eq!(result.as_i64(), Some(384));
}

// ===== Array methods =====

#[test]
fn test_typed_array_map() {
    let result = eval(
        "
        let arr = [1.0, 2.0, 3.0]
        let doubled = arr.map(|x| x * 2.0)
        doubled[2]
    ",
    );
    let val = result.to_number().expect("expected number");
    assert!((val - 6.0).abs() < 1e-10);
}

#[test]
fn test_typed_array_filter() {
    let result = eval(
        "
        let arr = [1, 2, 3, 4, 5, 6]
        let evens = arr.filter(|x| x % 2 == 0)
        evens.len()
    ",
    );
    assert_eq!(result.as_i64(), Some(3));
}

// ===== Error cases =====

#[test]
fn test_typed_array_out_of_bounds() {
    // Out-of-bounds access on v1 typed arrays returns none (not an error).
    let result = eval(
        "
        let arr = [1, 2, 3]
        arr[10]
    ",
    );
    assert!(result.is_none());
}

#[test]
fn test_typed_array_negative_index() {
    // Negative index should work (wraps around)
    let result = eval(
        "
        let arr = [10, 20, 30]
        arr[-1]
    ",
    );
    assert_eq!(result.as_i64(), Some(30));
}

// ===== Empty arrays =====

#[test]
fn test_empty_array_len() {
    let result = eval(
        "
        let arr: Array<int> = []
        arr.len()
    ",
    );
    assert_eq!(result.as_i64(), Some(0));
}

// ===== Mixed operations =====

#[test]
fn test_typed_array_dot_product() {
    let result = eval(
        "
        let a = [1.0, 2.0, 3.0]
        let b = [4.0, 5.0, 6.0]
        let mut dot = 0.0
        for i in 0..a.len() {
            dot = dot + a[i] * b[i]
        }
        dot
    ",
    );
    let val = result.to_number().expect("expected number");
    // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    assert!((val - 32.0).abs() < 1e-10);
}
