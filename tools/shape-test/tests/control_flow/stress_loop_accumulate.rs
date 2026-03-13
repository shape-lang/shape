//! Stress tests for complex loop accumulation patterns: if/else in loops, function
//! calls in loops, mutation, array building, algorithms (GCD, collatz, fibonacci,
//! binary search, integer sqrt), accumulators, sequential loops, and state machines.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 17. If/else inside loop body
// =========================================================================

/// Verifies if else in while.
#[test]
fn test_if_else_in_while() {
    ShapeTest::new("fn run() {\n    let mut even_sum = 0\n    let mut odd_sum = 0\n    let mut i = 1\n    while i <= 10 {\n        if i % 2 == 0 {\n            even_sum = even_sum + i\n        } else {\n            odd_sum = odd_sum + i\n        }\n        i = i + 1\n    }\n    even_sum - odd_sum\n}\nrun()").expect_number(5.0);
}

/// Verifies if else in for.
#[test]
fn test_if_else_in_for() {
    ShapeTest::new("fn run() {\n    let mut pos = 0\n    let mut neg = 0\n    for x in [5, -3, 2, -1, 4] {\n        if x > 0 {\n            pos = pos + x\n        } else {\n            neg = neg + x\n        }\n    }\n    pos + neg\n}\nrun()").expect_number(7.0);
}

// =========================================================================
// 18. Function calls inside loop
// =========================================================================

/// Verifies function call in while.
#[test]
fn test_function_call_in_while() {
    ShapeTest::new("fn double(x: int) -> int {\n    x * 2\n}\nfn run() {\n    let mut sum = 0\n    let mut i = 1\n    while i <= 5 {\n        sum = sum + double(i)\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(30.0);
}

/// Verifies function call in for.
#[test]
fn test_function_call_in_for() {
    ShapeTest::new("fn square(x: int) -> int {\n    x * x\n}\nfn run() {\n    let mut sum = 0\n    for x in [1, 2, 3, 4, 5] {\n        sum = sum + square(x)\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

// =========================================================================
// 19. Mutation in loop
// =========================================================================

/// Verifies mut variable increment.
#[test]
fn test_mut_variable_increment() {
    ShapeTest::new("fn run() {\n    let mut x = 0\n    let mut i = 0\n    while i < 20 {\n        x = x + 3\n        i = i + 1\n    }\n    x\n}\nrun()").expect_number(60.0);
}

/// Verifies mut multiple variables.
#[test]
fn test_mut_multiple_variables() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut b = 1\n    let mut i = 0\n    while i < 5 {\n        let temp = a\n        a = b\n        b = temp + b\n        i = i + 1\n    }\n    a\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 20. Array building in loop
// =========================================================================

/// Verifies push in while loop.
#[test]
fn test_push_in_while_loop() {
    ShapeTest::new("fn run() {\n    let mut out = []\n    let mut i = 0\n    while i < 5 {\n        out.push(i)\n        i = i + 1\n    }\n    len(out)\n}\nrun()").expect_number(5.0);
}

/// Verifies push in for loop.
#[test]
fn test_push_in_for_loop() {
    ShapeTest::new("fn run() {\n    let mut out = []\n    for x in [10, 20, 30] {\n        out.push(x)\n    }\n    len(out)\n}\nrun()").expect_number(3.0);
}

/// Verifies push conditional in loop.
#[test]
fn test_push_conditional_in_loop() {
    ShapeTest::new("fn run() {\n    let mut evens = []\n    for x in [1, 2, 3, 4, 5, 6, 7, 8] {\n        if x % 2 == 0 {\n            evens.push(x)\n        }\n    }\n    len(evens)\n}\nrun()").expect_number(4.0);
}

// =========================================================================
// 21. Complex accumulator patterns
// =========================================================================

/// Verifies GCD loop.
#[test]
fn test_gcd_loop() {
    ShapeTest::new("fn run() {\n    let mut a = 48\n    let mut b = 18\n    while b != 0 {\n        let temp = b\n        b = a % b\n        a = temp\n    }\n    a\n}\nrun()").expect_number(6.0);
}

/// Verifies collatz steps.
#[test]
fn test_collatz_steps() {
    ShapeTest::new("fn run() {\n    let mut n = 27\n    let mut steps = 0\n    while n != 1 {\n        if n % 2 == 0 {\n            n = n / 2\n        } else {\n            n = n * 3 + 1\n        }\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(111.0);
}

/// Verifies digit sum.
#[test]
fn test_digit_sum() {
    ShapeTest::new("fn run() {\n    let mut n = 12345\n    let mut sum = 0\n    while n > 0 {\n        sum = sum + n % 10\n        n = n / 10\n    }\n    sum\n}\nrun()").expect_number(15.0);
}

/// Verifies reverse number.
#[test]
fn test_reverse_number() {
    ShapeTest::new("fn run() {\n    let mut n = 1234\n    let mut rev = 0\n    while n > 0 {\n        rev = rev * 10 + n % 10\n        n = n / 10\n    }\n    rev\n}\nrun()").expect_number(4321.0);
}

// =========================================================================
// 22. Max/min finding in loop
// =========================================================================

/// Verifies find max in array.
#[test]
fn test_find_max_in_array() {
    ShapeTest::new("fn run() {\n    let mut max = -999999\n    for x in [3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5] {\n        if x > max {\n            max = x\n        }\n    }\n    max\n}\nrun()").expect_number(9.0);
}

/// Verifies find min in array.
#[test]
fn test_find_min_in_array() {
    ShapeTest::new("fn run() {\n    let mut min = 999999\n    for x in [3, 1, 4, 1, 5, 9, 2, 6] {\n        if x < min {\n            min = x\n        }\n    }\n    min\n}\nrun()").expect_number(1.0);
}

// =========================================================================
// 25. Fibonacci via loop
// =========================================================================

/// Verifies fibonacci 20th.
#[test]
fn test_fibonacci_20th() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut b = 1\n    let mut i = 0\n    while i < 20 {\n        let next = a + b\n        a = b\n        b = next\n        i = i + 1\n    }\n    a\n}\nrun()").expect_number(6765.0);
}

/// Verifies fibonacci first is zero.
#[test]
fn test_fibonacci_first_is_zero() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut b = 1\n    let mut i = 0\n    while i < 0 {\n        let next = a + b\n        a = b\n        b = next\n        i = i + 1\n    }\n    a\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 26. Large iteration counts
// =========================================================================

/// Verifies while 1000 iterations.
#[test]
fn test_while_1000_iterations() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 1000 {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(499500.0);
}

/// Verifies while 10000 iterations.
#[test]
fn test_while_10000_iterations() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 10000 {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(49995000.0);
}

// =========================================================================
// 27. Loop with let bindings in body
// =========================================================================

/// Verifies let in loop body.
#[test]
fn test_let_in_loop_body() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 5 {\n        let doubled = i * 2\n        sum = sum + doubled\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(20.0);
}

/// Verifies let in for body.
#[test]
fn test_let_in_for_body() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [1, 2, 3, 4, 5] {\n        let sq = x * x\n        let doubled = sq * 2\n        sum = sum + doubled\n    }\n    sum\n}\nrun()").expect_number(110.0);
}

// =========================================================================
// 31. Complex algorithm patterns
// =========================================================================

/// Verifies bubble sort check.
#[test]
fn test_bubble_sort_check() {
    ShapeTest::new("fn run() {\n    let mut passes = 0\n    let mut swapped = true\n    while swapped {\n        swapped = false\n        passes = passes + 1\n        if passes > 10 {\n            break\n        }\n    }\n    passes\n}\nrun()").expect_number(1.0);
}

/// Verifies binary search pattern.
#[test]
fn test_binary_search_pattern() {
    ShapeTest::new("fn run() {\n    let mut lo = 0\n    let mut hi = 100\n    let target = 73\n    let mut steps = 0\n    while lo < hi {\n        let mid = (lo + hi) / 2\n        steps = steps + 1\n        if mid < target {\n            lo = mid + 1\n        } else {\n            hi = mid\n        }\n    }\n    lo\n}\nrun()").expect_number(73.0);
}

/// Verifies integer sqrt.
#[test]
fn test_integer_sqrt() {
    ShapeTest::new("fn run() {\n    let n = 144\n    let mut guess = n / 2\n    let mut i = 0\n    while i < 100 {\n        let next = (guess + n / guess) / 2\n        if next == guess {\n            break\n        }\n        guess = next\n        i = i + 1\n    }\n    guess\n}\nrun()").expect_number(12.0);
}

// =========================================================================
// 36. Loop computing sum of geometric series
// =========================================================================

/// Verifies geometric series.
#[test]
fn test_geometric_series() {
    ShapeTest::new("fn run() {\n    let mut sum = 0.0\n    let mut term = 1.0\n    let mut i = 0\n    while i < 10 {\n        sum = sum + term\n        term = term / 2.0\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(1.998046875);
}

// =========================================================================
// 37. Loop with modular arithmetic
// =========================================================================

/// Verifies count divisible by 7.
#[test]
fn test_count_divisible_by_7() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 1\n    while i <= 100 {\n        if i % 7 == 0 {\n            count = count + 1\n        }\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(14.0);
}

// =========================================================================
// 39. Triangular numbers
// =========================================================================

/// Verifies triangular number.
#[test]
fn test_triangular_number() {
    ShapeTest::new("fn run() {\n    let n = 20\n    let mut sum = 0\n    let mut i = 1\n    while i <= n {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(210.0);
}

// =========================================================================
// 43. Loop with multiple assignments per iteration
// =========================================================================

/// Verifies swap in loop.
#[test]
fn test_swap_in_loop() {
    ShapeTest::new("fn run() {\n    let mut a = 1\n    let mut b = 0\n    let mut i = 0\n    while i < 6 {\n        let temp = a\n        a = b\n        b = temp\n        i = i + 1\n    }\n    a\n}\nrun()").expect_number(1.0);
}

// =========================================================================
// 46. Loop building a running max
// =========================================================================

/// Verifies running max.
#[test]
fn test_running_max() {
    ShapeTest::new("fn run() {\n    let mut max = 0\n    let values = [3, 7, 2, 8, 1, 9, 4, 6, 5]\n    for v in values {\n        if v > max {\n            max = v\n        }\n    }\n    max\n}\nrun()").expect_number(9.0);
}

// =========================================================================
// 47. Successive approximation
// =========================================================================

/// Verifies newton sqrt approximation.
#[test]
fn test_newton_sqrt_approximation() {
    ShapeTest::new("fn run() {\n    let n = 2.0\n    let mut guess = 1.0\n    let mut i = 0\n    while i < 20 {\n        guess = (guess + n / guess) / 2.0\n        i = i + 1\n    }\n    guess\n}\nrun()").expect_number(1.4142135623730951);
}

// =========================================================================
// 49. Multiple loops sequential
// =========================================================================

/// Verifies sequential loops.
#[test]
fn test_sequential_loops() {
    ShapeTest::new("fn run() {\n    let mut sum1 = 0\n    for x in [1, 2, 3] {\n        sum1 = sum1 + x\n    }\n    let mut sum2 = 0\n    for y in [10, 20, 30] {\n        sum2 = sum2 + y\n    }\n    sum1 + sum2\n}\nrun()").expect_number(66.0);
}

/// Verifies three sequential while loops.
#[test]
fn test_three_sequential_while_loops() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut i = 0\n    while i < 3 { a = a + 1; i = i + 1 }\n    let mut b = 0\n    i = 0\n    while i < 4 { b = b + 1; i = i + 1 }\n    let mut c = 0\n    i = 0\n    while i < 5 { c = c + 1; i = i + 1 }\n    a + b + c\n}\nrun()").expect_number(12.0);
}

// =========================================================================
// 50. Power computation
// =========================================================================

/// Verifies power loop.
#[test]
fn test_power_loop() {
    ShapeTest::new("fn run() {\n    let base = 3\n    let exp = 7\n    let mut result = 1\n    let mut i = 0\n    while i < exp {\n        result = result * base\n        i = i + 1\n    }\n    result\n}\nrun()").expect_number(2187.0);
}

// =========================================================================
// 51. Loop with nested if/else chains
// =========================================================================

/// Verifies classify in loop.
#[test]
fn test_classify_in_loop() {
    ShapeTest::new("fn run() {\n    let mut small = 0\n    let mut medium = 0\n    let mut large = 0\n    for x in [1, 5, 15, 3, 50, 8, 100, 2] {\n        if x < 5 {\n            small = small + 1\n        } else if x < 20 {\n            medium = medium + 1\n        } else {\n            large = large + 1\n        }\n    }\n    small * 100 + medium * 10 + large\n}\nrun()").expect_number(332.0);
}

// =========================================================================
// 59. Loop accumulator with subtraction
// =========================================================================

/// Verifies alternating sum loop.
#[test]
fn test_alternating_sum_loop() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut sign = 1\n    let mut i = 1\n    while i <= 10 {\n        sum = sum + sign * i\n        sign = sign * -1\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(-5.0);
}

// =========================================================================
// 68. For loop: sum of first N odd numbers
// =========================================================================

/// Verifies sum first n odd.
#[test]
fn test_sum_first_n_odd() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut count = 0\n    let mut i = 1\n    while count < 10 {\n        sum = sum + i\n        i = i + 2\n        count = count + 1\n    }\n    sum\n}\nrun()").expect_number(100.0);
}

// =========================================================================
// 69. Loop computing harmonic sum
// =========================================================================

/// Verifies harmonic sum.
#[test]
fn test_harmonic_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0.0\n    let mut i = 1\n    while i <= 10 {\n        sum = sum + 1.0 / i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(2.9289682539682538);
}

// =========================================================================
// 72. Loop with boolean accumulation
// =========================================================================

/// Verifies all positive.
#[test]
fn test_all_positive() {
    ShapeTest::new("fn run() {\n    let mut all_pos = true\n    for x in [1, 2, 3, 4, 5] {\n        if x <= 0 {\n            all_pos = false\n        }\n    }\n    if all_pos { 1 } else { 0 }\n}\nrun()").expect_number(1.0);
}

/// Verifies not all positive.
#[test]
fn test_not_all_positive() {
    ShapeTest::new("fn run() {\n    let mut all_pos = true\n    for x in [1, 2, -3, 4] {\n        if x <= 0 {\n            all_pos = false\n        }\n    }\n    if all_pos { 1 } else { 0 }\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 75. Loop with two accumulators
// =========================================================================

/// Verifies two accumulators.
#[test]
fn test_two_accumulators() {
    ShapeTest::new("fn run() {\n    let mut sum_even = 0\n    let mut sum_odd = 0\n    let mut i = 1\n    while i <= 10 {\n        if i % 2 == 0 {\n            sum_even = sum_even + i\n        } else {\n            sum_odd = sum_odd + i\n        }\n        i = i + 1\n    }\n    sum_even * sum_odd\n}\nrun()").expect_number(750.0);
}

// =========================================================================
// 78. Loop computing average
// =========================================================================

/// Verifies average of values.
#[test]
fn test_average_of_values() {
    ShapeTest::new("fn run() {\n    let mut sum = 0.0\n    let mut count = 0\n    for x in [10, 20, 30, 40, 50] {\n        sum = sum + x\n        count = count + 1\n    }\n    sum / count\n}\nrun()").expect_number(30.0);
}

// =========================================================================
// 79. While loop: bit counting
// =========================================================================

/// Verifies count bits.
#[test]
fn test_count_bits() {
    ShapeTest::new("fn run() {\n    let mut n = 255\n    let mut bits = 0\n    while n > 0 {\n        bits = bits + n % 2\n        n = n / 2\n    }\n    bits\n}\nrun()").expect_number(8.0);
}

// =========================================================================
// 80. For loop: index tracking
// =========================================================================

/// Verifies for index tracking.
#[test]
fn test_for_index_tracking() {
    ShapeTest::new("fn run() {\n    let mut weighted_sum = 0\n    let mut idx = 0\n    for x in [10, 20, 30] {\n        weighted_sum = weighted_sum + x * idx\n        idx = idx + 1\n    }\n    weighted_sum\n}\nrun()").expect_number(80.0);
}

// =========================================================================
// 83. Loop with string building
// =========================================================================

/// Verifies string char count in loop.
#[test]
#[should_panic]
fn test_string_char_count_in_loop() {
    ShapeTest::new("fn run() {\n    let mut count_a = 0\n    for ch in \"banana\" {\n        if ch == \"a\" {\n            count_a = count_a + 1\n        }\n    }\n    count_a\n}\nrun()").expect_number(3.0);
}

// =========================================================================
// 87. Loop: XOR all elements
// =========================================================================

/// Verifies xor accumulate.
#[test]
fn test_xor_accumulate() {
    ShapeTest::new("fn run() {\n    let mut xor = 0\n    for x in [1, 2, 3, 4, 5, 6, 7, 8] {\n        xor = xor ^ x\n    }\n    xor\n}\nrun()").expect_number(8.0);
}

// =========================================================================
// 91. Sequential for and while
// =========================================================================

/// Verifies for then while.
#[test]
fn test_for_then_while() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [1, 2, 3] {\n        sum = sum + x\n    }\n    let mut i = 4\n    while i <= 6 {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(21.0);
}

// =========================================================================
// 93. Loop: sum only positive values
// =========================================================================

/// Verifies sum positive only.
#[test]
fn test_sum_positive_only() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [3, -2, 7, -5, 1, -8, 4] {\n        if x > 0 {\n            sum = sum + x\n        }\n    }\n    sum\n}\nrun()").expect_number(15.0);
}

// =========================================================================
// 94. Loop computing parity
// =========================================================================

/// Verifies parity loop.
#[test]
fn test_parity_loop() {
    ShapeTest::new("fn run() {\n    let mut parity = 0\n    for x in [1, 0, 1, 1, 0, 1] {\n        parity = parity ^ x\n    }\n    parity\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 95. Loop: compute absolute difference
// =========================================================================

/// Verifies absolute difference sum.
#[test]
fn test_absolute_difference_sum() {
    ShapeTest::new("fn run() {\n    let a = [1, 5, 3, 7]\n    let b = [2, 3, 6, 1]\n    let mut diff = 0\n    let mut i = 0\n    while i < 4 {\n        let d = a[i] - b[i]\n        if d < 0 {\n            diff = diff - d\n        } else {\n            diff = diff + d\n        }\n        i = i + 1\n    }\n    diff\n}\nrun()").expect_number(12.0);
}

// =========================================================================
// 97. Loop: compute dot product
// =========================================================================

/// Verifies dot product.
#[test]
fn test_dot_product() {
    ShapeTest::new("fn run() {\n    let a = [1, 2, 3, 4]\n    let b = [5, 6, 7, 8]\n    let mut dot = 0\n    let mut i = 0\n    while i < 4 {\n        dot = dot + a[i] * b[i]\n        i = i + 1\n    }\n    dot\n}\nrun()").expect_number(70.0);
}

// =========================================================================
// 98. Loop calling recursive function
// =========================================================================

/// Verifies loop calling recursive fn.
#[test]
fn test_loop_calling_recursive_fn() {
    ShapeTest::new("fn factorial(n: int) -> int {\n    if n <= 1 { return 1 }\n    n * factorial(n - 1)\n}\nfn run() {\n    let mut sum = 0\n    for i in [1, 2, 3, 4, 5] {\n        sum = sum + factorial(i)\n    }\n    sum\n}\nrun()").expect_number(153.0);
}

// =========================================================================
// 99. Sieve-like pattern
// =========================================================================

/// Verifies count non-multiples.
#[test]
fn test_count_non_multiples() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 1\n    while i <= 30 {\n        if i % 2 != 0 && i % 3 != 0 && i % 5 != 0 {\n            count = count + 1\n        }\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(8.0);
}

// =========================================================================
// 101. Loop: cumulative sum
// =========================================================================

/// Verifies cumulative sum array.
#[test]
fn test_cumulative_sum_array() {
    ShapeTest::new("fn run() {\n    let data = [1, 2, 3, 4, 5]\n    let mut cumsum = []\n    let mut running = 0\n    for x in data {\n        running = running + x\n        cumsum.push(running)\n    }\n    len(cumsum)\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 104. For loop: count matching pairs
// =========================================================================

/// Verifies count equal adjacent.
#[test]
fn test_count_equal_adjacent() {
    ShapeTest::new("fn run() {\n    let arr = [1, 1, 2, 3, 3, 3, 4, 4]\n    let mut pairs = 0\n    let mut i = 0\n    while i < 7 {\n        if arr[i] == arr[i + 1] {\n            pairs = pairs + 1\n        }\n        i = i + 1\n    }\n    pairs\n}\nrun()").expect_number(4.0);
}

// =========================================================================
// 108. For loop with complex body: multiple let bindings
// =========================================================================

/// Verifies for complex body.
#[test]
fn test_for_complex_body() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    for x in [2, 4, 6, 8] {\n        let half = x / 2\n        let plus_one = half + 1\n        let squared = plus_one * plus_one\n        total = total + squared\n    }\n    total\n}\nrun()").expect_number(54.0);
}

// =========================================================================
// 109. Loop: minimum absolute value
// =========================================================================

/// Verifies min absolute.
#[test]
fn test_min_absolute() {
    ShapeTest::new("fn run() {\n    let mut min_abs = 999999\n    for x in [-10, 5, -2, 8, -1, 3] {\n        let abs_x = if x < 0 { -x } else { x }\n        if abs_x < min_abs {\n            min_abs = abs_x\n        }\n    }\n    min_abs\n}\nrun()").expect_number(1.0);
}

// =========================================================================
// 110. Loop: simulate state machine
// =========================================================================

/// Verifies state machine loop.
#[test]
fn test_state_machine_loop() {
    ShapeTest::new("fn run() {\n    let mut state = 0\n    let mut transitions = 0\n    let inputs = [1, 0, 1, 1, 0, 0, 1]\n    for inp in inputs {\n        if state == 0 && inp == 1 {\n            state = 1\n            transitions = transitions + 1\n        } else if state == 1 && inp == 0 {\n            state = 0\n            transitions = transitions + 1\n        }\n    }\n    transitions\n}\nrun()").expect_number(5.0);
}
