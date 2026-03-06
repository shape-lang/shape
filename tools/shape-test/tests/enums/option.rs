use shape_test::shape_test::ShapeTest;

// =========================================================================
// Option<T> — Built-in Generic Enum
// =========================================================================

#[test]
fn builtin_option_some_and_none() {
    let code = r#"
let a: Option<int> = Some(42)
let b: Option<int> = None
print(a)
print(b)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("42");
}

// =========================================================================
// Option<T> — Construction and Basic Usage
// =========================================================================

// BUG: Some(42) prints as "42" -- Some is transparent in printing
#[test]
fn test_option_some_construction() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        print(x)
    "#,
    )
    .expect_output_contains("42");
}

#[test]
fn test_option_none_construction() {
    ShapeTest::new(
        r#"
        let x = None
        print(x)
    "#,
    )
    .expect_run_ok();
}

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context.
// These are kept as known-bug markers; when the compiler is fixed they should pass.

#[test]
fn test_option_match_some() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x {
            Some(val) => val,
            None => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_option_match_none() {
    ShapeTest::new(
        r#"
        let x = None
        match x {
            Some(val) => val,
            None => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_option_as_function_return_some() {
    ShapeTest::new(
        r#"
        fn find_value(flag) {
            if flag { Some(99) } else { None }
        }
        match find_value(true) {
            Some(v) => v,
            None => 0
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn test_option_as_function_return_none() {
    ShapeTest::new(
        r#"
        fn find_value(flag) {
            if flag { Some(99) } else { None }
        }
        match find_value(false) {
            Some(v) => v,
            None => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_option_some_with_string() {
    ShapeTest::new(
        r#"
        let x = Some("hello")
        match x {
            Some(s) => s,
            None => "empty"
        }
    "#,
    )
    .expect_string("hello");
}

#[test]
fn test_option_some_with_bool() {
    ShapeTest::new(
        r#"
        let x = Some(true)
        match x {
            Some(b) => b,
            None => false
        }
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_option_none_default_string() {
    ShapeTest::new(
        r#"
        let x = None
        match x {
            Some(s) => s,
            None => "default"
        }
    "#,
    )
    .expect_string("default");
}

#[test]
fn test_option_in_conditional() {
    ShapeTest::new(
        r#"
        fn maybe(n) {
            if n > 0 { Some(n * 10) } else { None }
        }
        let a = match maybe(5) {
            Some(v) => v,
            None => 0
        }
        let b = match maybe(-1) {
            Some(v) => v,
            None => 0
        }
        a + b
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn test_option_some_with_zero() {
    ShapeTest::new(
        r#"
        let x = Some(0)
        match x {
            Some(val) => val + 1,
            None => -1
        }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_option_in_array() {
    ShapeTest::new(
        r#"
        let opts = [Some(1), None, Some(3)]
        let sum = 0
        for opt in opts {
            sum = sum + match opt {
                Some(v) => v,
                None => 0
            }
        }
        sum
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_option_some_with_negative() {
    ShapeTest::new(
        r#"
        let x = Some(-42)
        match x {
            Some(val) => val,
            None => 0
        }
    "#,
    )
    .expect_number(-42.0);
}

#[test]
fn test_option_match_with_computation() {
    ShapeTest::new(
        r#"
        let x = Some(10)
        match x {
            Some(v) => v * v,
            None => 0
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_option_none_comparison() {
    ShapeTest::new(
        r#"
        let a = None
        let b = None
        // Both are None
        let ra = match a { Some(v) => v, None => -1 }
        let rb = match b { Some(v) => v, None => -1 }
        ra == rb
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// Option<T> — From Flat File (wildcard-based patterns)
// =========================================================================

#[test]
fn option_some_number() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        x
    "#,
    )
    .expect_run_ok();
}

#[test]
fn option_none_value() {
    ShapeTest::new(
        r#"
        let x = None
        x
    "#,
    )
    .expect_run_ok();
}

#[test]
fn option_some_string_via_wildcard() {
    ShapeTest::new(
        r#"
        let x = Some("hello")
        match x {
            Some(v) => v,
            _ => "nothing"
        }
    "#,
    )
    .expect_string("hello");
}

#[test]
fn option_some_bool_via_wildcard() {
    ShapeTest::new(
        r#"
        let x = Some(true)
        match x {
            Some(v) => v,
            _ => false
        }
    "#,
    )
    .expect_bool(true);
}

#[test]
fn option_match_some_extracts_number() {
    ShapeTest::new(
        r#"
        let opt = Some(42)
        match opt {
            Some(val) => val,
            _ => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn option_match_none_takes_default() {
    ShapeTest::new(
        r#"
        let opt = None
        match opt {
            Some(val) => val,
            _ => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn option_in_variable_then_match() {
    ShapeTest::new(
        r#"
        let x = Some(100)
        let y = match x {
            Some(v) => v + 1,
            _ => 0
        }
        y
    "#,
    )
    .expect_number(101.0);
}

#[test]
fn option_from_function_return_some() {
    ShapeTest::new(
        r#"
        fn find_value(n) {
            if n > 0 { Some(n * 10) } else { None }
        }
        match find_value(5) {
            Some(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn option_from_function_return_none() {
    ShapeTest::new(
        r#"
        fn find_value(n) {
            if n > 0 { Some(n * 10) } else { None }
        }
        match find_value(-1) {
            Some(v) => v,
            _ => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn option_some_with_expression_payload() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        let opt = Some(a + b)
        match opt {
            Some(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn option_none_match_returns_string() {
    ShapeTest::new(
        r#"
        let opt = None
        match opt {
            Some(v) => "found",
            _ => "not found"
        }
    "#,
    )
    .expect_string("not found");
}

#[test]
fn option_some_match_returns_string() {
    ShapeTest::new(
        r#"
        let opt = Some("world")
        match opt {
            Some(v) => "hello " + v,
            _ => "nobody"
        }
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn option_some_zero_is_some_not_none() {
    ShapeTest::new(
        r#"
        let opt = Some(0)
        match opt {
            Some(v) => "is some",
            _ => "is none"
        }
    "#,
    )
    .expect_string("is some");
}

#[test]
fn option_some_false_is_some_not_none() {
    ShapeTest::new(
        r#"
        let opt = Some(false)
        match opt {
            Some(v) => "is some",
            _ => "is none"
        }
    "#,
    )
    .expect_string("is some");
}

#[test]
fn option_some_empty_string_is_some() {
    ShapeTest::new(
        r#"
        let opt = Some("")
        match opt {
            Some(v) => "is some",
            _ => "is none"
        }
    "#,
    )
    .expect_string("is some");
}

// BUG: print(Some(42)) prints "42" instead of "Some(42)" -- value is unwrapped
#[test]
fn option_print_some_unwraps() {
    ShapeTest::new(
        r#"
        let opt = Some(42)
        print(opt)
    "#,
    )
    .expect_output("42");
}

#[test]
fn option_print_none() {
    ShapeTest::new(
        r#"
        let opt = None
        print(opt)
    "#,
    )
    .expect_output("None");
}

#[test]
fn option_match_used_as_expression() {
    ShapeTest::new(
        r#"
        let result = match Some(7) {
            Some(v) => v * v,
            _ => 0
        }
        result
    "#,
    )
    .expect_number(49.0);
}

#[test]
fn option_in_conditional_construction() {
    ShapeTest::new(
        r#"
        let flag = true
        let opt = if flag { Some(1) } else { None }
        match opt {
            Some(v) => v,
            _ => -1
        }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn option_none_in_conditional_construction() {
    ShapeTest::new(
        r#"
        let flag = false
        let opt = if flag { Some(1) } else { None }
        match opt {
            Some(v) => v,
            _ => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Option<T> — Complex Programs
// =========================================================================

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_complex_option_chain_lookup() {
    ShapeTest::new(
        r#"
        fn lookup(key) {
            if key == "a" { Some(1) }
            else if key == "b" { Some(2) }
            else { None }
        }
        let total = 0
        let keys = ["a", "b", "c", "a"]
        for k in keys {
            total = total + match lookup(k) {
                Some(v) => v,
                None => 0
            }
        }
        total
    "#,
    )
    .expect_number(4.0);
}

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_complex_accumulate_with_option() {
    ShapeTest::new(
        r#"
        fn safe_get(arr, idx) {
            if idx >= 0 and idx < arr.length {
                Some(arr[idx])
            } else {
                None
            }
        }
        let data = [10, 20, 30, 40, 50]
        let sum = 0
        for i in [0, 2, 4, 6, 8] {
            sum = sum + match safe_get(data, i) {
                Some(v) => v,
                None => 0
            }
        }
        sum
    "#,
    )
    .expect_number(90.0);
}

// =========================================================================
// Option<T> — BUG Tests: Bare None Patterns
// =========================================================================

// Bare Some/None patterns work on untyped variables (fixed).
#[test]
fn bug_bare_none_pattern_on_untyped_var() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x {
            Some(val) => val,
            None => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn bug_bare_none_pattern_on_none_var() {
    ShapeTest::new(
        r#"
        let x = None
        match x {
            Some(val) => val,
            None => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Option<T> — In Multiple Functions
// =========================================================================

#[test]
fn option_match_in_multiple_functions() {
    ShapeTest::new(
        r#"
        fn safe_head(arr) {
            if arr.length == 0 { None } else { Some(arr[0]) }
        }
        fn double_head(arr) {
            match safe_head(arr) {
                Some(v) => v * 2,
                _ => 0
            }
        }
        double_head([5, 10, 15])
    "#,
    )
    .expect_number(10.0);
}
