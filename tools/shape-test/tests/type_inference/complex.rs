//! Complex program tests — fibonacci, prime checking, sorting,
//! transformation chains, and mixed-type data processing.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 6. Complex Programs (10 tests)
// =========================================================================

#[test]
fn test_complex_fibonacci_iterative() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            var a = 0
            var b = 1
            for i in 0..n {
                let temp = b
                b = a + b
                a = temp
            }
            a
        }
        fib(10)
    "#,
    )
    .expect_number(55.0);
}

#[test]
fn test_complex_is_prime() {
    ShapeTest::new(
        r#"
        fn is_prime(n) {
            if n < 2 { return false }
            var i = 2
            while i * i <= n {
                if n % i == 0 { return false }
                i = i + 1
            }
            true
        }
        print(is_prime(2))
        print(is_prime(7))
        print(is_prime(10))
        print(is_prime(13))
    "#,
    )
    .expect_output("true\ntrue\nfalse\ntrue");
}

#[test]
fn test_complex_array_transformation_chain() {
    // Filter evens, double them, sum
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 2)
            .reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_complex_string_processing_pipeline() {
    ShapeTest::new(
        r#"
        let words = "hello world foo bar".split(" ")
        let upper = words.map(|w| w.toUpperCase())
        upper.join(", ")
    "#,
    )
    .expect_string("HELLO, WORLD, FOO, BAR");
}

#[test]
fn test_complex_bubble_sort() {
    ShapeTest::new(
        r#"
        fn bubble_sort(arr) {
            var n = arr.length
            var sorted = arr
            var i = 0
            while i < n {
                var j = 0
                while j < n - 1 - i {
                    if sorted[j] > sorted[j + 1] {
                        let temp = sorted[j]
                        sorted = sorted.slice(0, j)
                            .concat([sorted[j + 1]])
                            .concat([temp])
                            .concat(sorted.slice(j + 2, n))
                    }
                    j = j + 1
                }
                i = i + 1
            }
            sorted
        }
        let result = bubble_sort([5, 3, 8, 1, 2])
        print(result[0])
        print(result[1])
        print(result[2])
        print(result[3])
        print(result[4])
    "#,
    )
    .expect_output("1\n2\n3\n5\n8");
}

#[test]
fn test_complex_factorial_iterative() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            var result = 1
            var i = 1
            while i <= n {
                result = result * i
                i = i + 1
            }
            result
        }
        factorial(10)
    "#,
    )
    .expect_number(3628800.0);
}

#[test]
fn test_complex_gcd_euclidean() {
    ShapeTest::new(
        r#"
        fn gcd(a, b) {
            var x = a
            var y = b
            while y != 0 {
                let temp = y
                y = x % y
                x = temp
            }
            x
        }
        gcd(48, 18)
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_accumulate_with_hashmap() {
    ShapeTest::new(
        r#"
        let m = HashMap()
            .set("apples", 3)
            .set("bananas", 5)
            .set("oranges", 2)
        let total = m.get("apples") + m.get("bananas") + m.get("oranges")
        total
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_complex_nested_array_flatten() {
    ShapeTest::new(
        r#"
        let nested = [[1, 2], [3, 4], [5, 6]]
        let flat = nested.flatten()
        flat.reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn test_complex_data_processing_mixed_types() {
    ShapeTest::new(
        r#"
        let names = ["Alice", "Bob", "Charlie", "Diana"]
        let greeting = names
            .filter(|n| n.length > 3)
            .map(|n| "Hello " + n)
            .join("; ")
        greeting
    "#,
    )
    .expect_string("Hello Alice; Hello Charlie; Hello Diana");
}
