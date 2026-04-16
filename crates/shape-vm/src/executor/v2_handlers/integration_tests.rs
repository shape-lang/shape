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
use shape_value::ValueWordExt;

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
    // v2 STRICT semantics: out-of-bounds index access raises a runtime error
    // through the `TypedArrayGetI64` fast-path opcode. (Wave B made typed
    // emission unconditional, so the legacy lenient `GetProp` path is no
    // longer reachable for inferable element types.)
    let result = crate::test_utils::eval_result(
        "
        let arr = [1, 2, 3]
        arr[10]
    ",
    );
    assert!(result.is_err(), "expected IndexOutOfBounds, got {:?}", result);
}

#[test]
fn test_typed_array_negative_index() {
    // v2 STRICT semantics: negative index raises a runtime error.
    // The lenient wrap-around behavior was a legacy v1 feature provided by
    // generic `GetProp`; the v2 `TypedArrayGetI64` opcode does not wrap.
    let result = crate::test_utils::eval_result(
        "
        let arr = [10, 20, 30]
        arr[-1]
    ",
    );
    assert!(result.is_err(), "expected IndexOutOfBounds, got {:?}", result);
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

// ===== New end-to-end v2 typed array demos (Phase 4 Agent 2) =====
//
// These three tests demonstrate end-to-end v2 typed array usage against
// the consumer-side dispatch paths wired up in `v2_array_detect`:
//
//   1. `arr.length` (GetProp "length") returns the v2 header length directly.
//   2. `for x in arr { ... }` iterates through the typed element buffer.
//   3. `arr[0] = 99.0; arr[0]` mutates and reads back via typed set/get.

#[test]
fn test_v2_typed_array_length_property() {
    // Array<number> literal is lowered to v2 TypedArray<f64>. The
    // `.length` property access goes through `op_get_prop`, which
    // recognises the v2 typed array and returns the stamped header len.
    let result = eval(
        "
        let arr: Array<number> = [1.0, 2.0, 3.0]
        arr.length
    ",
    );
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_v2_typed_array_for_in_iteration() {
    // Array<number> literal is lowered to v2 TypedArray<f64>. The
    // `for x in arr` loop dispatches to `op_iter_next`/`op_iter_done`,
    // which now read elements through the v2 header.
    let result = eval(
        "
        let arr: Array<number> = [1.0, 2.0, 3.0]
        let mut total = 0.0
        for x in arr {
            total = total + x
        }
        total
    ",
    );
    let val = result.to_number().expect("expected number");
    assert!((val - 6.0).abs() < 1e-10);
}

#[test]
fn test_v2_typed_array_index_assignment_roundtrip() {
    // `arr[0] = 99.0; arr[0]` mutates and reads back through the v2
    // typed array fast path. The compiler may emit either
    // `TypedArraySetF64`/`TypedArrayGetF64` (compile-time tracked) or
    // the generic `SetProp`/`GetProp` route — both paths now recognise
    // the v2 pointer via the stamped header.
    let result = eval(
        "
        let mut arr: Array<number> = [1.0, 2.0, 3.0]
        arr[0] = 99.0
        arr[0]
    ",
    );
    let val = result.to_number().expect("expected number");
    assert!((val - 99.0).abs() < 1e-10);
}
