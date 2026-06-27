//! Stress & Edge Cases (10 tests)
//!
//! Tests for many variables, deeply nested expressions, long method chains,
//! large strings, many function definitions, and comprehensive feature combinations.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Stress & Edge Cases (10 tests)
// =========================================================================

#[test]
fn test_complex_many_variables_in_scope() {
    ShapeTest::new(
        r#"
        let v01 = 1; let v02 = 2; let v03 = 3; let v04 = 4; let v05 = 5
        let v06 = 6; let v07 = 7; let v08 = 8; let v09 = 9; let v10 = 10
        let v11 = 11; let v12 = 12; let v13 = 13; let v14 = 14; let v15 = 15
        let v16 = 16; let v17 = 17; let v18 = 18; let v19 = 19; let v20 = 20
        let v21 = 21; let v22 = 22; let v23 = 23; let v24 = 24; let v25 = 25
        let v26 = 26; let v27 = 27; let v28 = 28; let v29 = 29; let v30 = 30
        let v31 = 31; let v32 = 32; let v33 = 33; let v34 = 34; let v35 = 35
        let v36 = 36; let v37 = 37; let v38 = 38; let v39 = 39; let v40 = 40
        let v41 = 41; let v42 = 42; let v43 = 43; let v44 = 44; let v45 = 45
        let v46 = 46; let v47 = 47; let v48 = 48; let v49 = 49; let v50 = 50
        v01 + v10 + v20 + v30 + v40 + v50
    "#,
    )
    .expect_number(151.0);
}

#[test]
fn test_complex_deeply_nested_expressions() {
    // (1+2)=3, *3=9, -4=5, +5=10, *2=20, -1=19, +3=22, *2=44, -10=34
    ShapeTest::new(
        r#"
        let result = (((((((((1 + 2) * 3) - 4) + 5) * 2) - 1) + 3) * 2) - 10)
        result
    "#,
    )
    .expect_number(34.0);
}

#[test]
fn test_complex_long_method_chain() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .map(|x| x * 2)
            .filter(|x| x > 5)
            .map(|x| x + 1)
            .filter(|x| x < 20)
            .map(|x| x - 1)
            .reduce(|acc, x| acc + x, 0)
        result
    "#,
    )
    .expect_number(84.0);
}

#[test]
fn test_complex_large_string_operations() {
    ShapeTest::new(
        r#"
        let mut s = ""
        let mut i = 0
        while i < 100 {
            s = s + "x"
            i = i + 1
        }
        print(s.length)
        print(s.contains("xxx"))
    "#,
    )
    .expect_output("100\ntrue");
}

#[test]
fn test_complex_many_function_definitions() {
    ShapeTest::new(
        r#"
        fn f01(x) { x + 1 }
        fn f02(x) { x + 2 }
        fn f03(x) { x + 3 }
        fn f04(x) { x + 4 }
        fn f05(x) { x + 5 }
        fn f06(x) { x + 6 }
        fn f07(x) { x + 7 }
        fn f08(x) { x + 8 }
        fn f09(x) { x + 9 }
        fn f10(x) { x + 10 }
        fn f11(x) { x + 11 }
        fn f12(x) { x + 12 }
        fn f13(x) { x + 13 }
        fn f14(x) { x + 14 }
        fn f15(x) { x + 15 }
        fn f16(x) { x + 16 }
        fn f17(x) { x + 17 }
        fn f18(x) { x + 18 }
        fn f19(x) { x + 19 }
        fn f20(x) { x + 20 }
        f01(0) + f05(0) + f10(0) + f15(0) + f20(0)
    "#,
    )
    .expect_number(51.0);
}

#[test]
fn test_complex_nested_control_flow() {
    ShapeTest::new(
        r#"
        fn process(arr) {
            let mut result = 0
            for item in arr {
                if item > 0 {
                    match item {
                        n where n > 50 => {
                            let mut i = 0
                            while i < 3 {
                                result = result + n
                                i = i + 1
                            }
                        },
                        n where n > 10 => {
                            result = result + n * 2
                        },
                        _ => {
                            result = result + item
                        }
                    }
                }
            }
            result
        }
        process([5, 15, 60, -10, 25, 3])
    "#,
    )
    .expect_number(268.0);
}

#[test]
fn test_complex_all_features_together() {
    ShapeTest::new(
        r#"
        // Types
        type Item { name: string, price: number }
        // Enum
        enum Discount { Percent(number), Fixed(number), NoDiscount }
        // Functions
        fn apply_discount(price, discount) {
            match discount {
                Discount::Percent(p) => price * (100 - p) / 100,
                Discount::Fixed(f) => price - f,
                Discount::NoDiscount => price
            }
        }
        // Const
        const TAX_RATE = 10
        fn apply_tax(price) { price * (100 + TAX_RATE) / 100 }
        // Array + closures + iteration
        let items = [
            Item { name: "Widget", price: 100 },
            Item { name: "Gadget", price: 200 },
            Item { name: "Doohickey", price: 50 }
        ]
        let discount = Discount::Percent(20)
        let mut total = 0
        for item in items {
            let discounted = apply_discount(item.price, discount)
            let with_tax = apply_tax(discounted)
            total = total + with_tax
        }
        // HashMap for metadata
        let receipt = HashMap()
            .set("items", items.length)
            .set("total", total)
        print(receipt.get("items"))
        print(receipt.get("total"))
    "#,
    )
    .expect_output("3\n308.0");
}

#[test]
fn test_complex_deeply_nested_closures() {
    // Deep nesting with explicit intermediate bindings
    ShapeTest::new(
        r#"
        fn level1(a) {
            |b| {
                |c| a + b + c
            }
        }
        let l2 = level1(1)
        let l3 = l2(2)
        l3(3)
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_recursive_descent_evaluator() {
    ShapeTest::new(
        r#"
        // Simple postfix expression evaluator
        fn eval_postfix(tokens) {
            let mut stack = []
            for t in tokens {
                match t {
                    "+" => {
                        let b = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        let a = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        stack = stack.push(a + b)
                    },
                    "*" => {
                        let b = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        let a = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        stack = stack.push(a * b)
                    },
                    n => {
                        stack = stack.push(n)
                    }
                }
            }
            stack[0]
        }
        // 3 4 + 2 * = (3 + 4) * 2 = 14
        eval_postfix([3, 4, "+", 2, "*"])
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn test_complex_large_program_with_everything() {
    ShapeTest::new(
        r#"
        // Enum
        enum Status { Active, Inactive, Pending }
        // Type
        type User { name: string, age: int, status: Status }
        // Extend
        extend User {
            method is_active() { self.status == Status::Active }
        }
        // Functions
        fn make_user(name, age, active) {
            User {
                name: name,
                age: age,
                status: if active { Status::Active } else { Status::Inactive }
            }
        }
        fn count_active(users) {
            users.filter(|u| u.is_active()).length
        }
        fn average_age(users) {
            let total = users.map(|u| u.age).reduce(|acc, x| acc + x, 0)
            total / users.length
        }
        fn oldest(users) {
            users.reduce(|best, u| if u.age > best.age { u } else { best }, users[0])
        }
        // Data
        let users = [
            make_user("Alice", 30, true),
            make_user("Bob", 25, false),
            make_user("Charlie", 35, true),
            make_user("Diana", 28, true)
        ]
        // Output
        print(count_active(users))
        print(average_age(users))
        print(oldest(users).name)
        // Closures
        let active_names = users
            .filter(|u| u.is_active())
            .map(|u| u.name)
        for name in active_names { print(name) }
    "#,
    )
    .expect_output("3\n29\nCharlie\nAlice\nCharlie\nDiana");
}

#[test]
fn test_complex_reduce_with_initial_and_transform() {
    // Combine reduce, map, filter, closures, and functions in one pipeline
    ShapeTest::new(
        r#"
        fn square(x) { x * x }
        let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        let result = numbers
            .filter(|x| x % 2 != 0)
            .map(|x| square(x))
            .reduce(|acc, x| acc + x, 0)
        // 1 + 9 + 25 + 49 + 81 = 165
        result
    "#,
    )
    .expect_number(165.0);
}

#[test]
fn test_complex_enum_dispatch_with_closures_and_loop() {
    // Combines enum, match, closures, and loops in a mini-scheduler
    ShapeTest::new(
        r#"
        enum Task { Compute(int), Log(string), Halt }
        fn run_tasks(tasks) {
            let mut total = 0
            for task in tasks {
                match task {
                    Task::Compute(n) => { total = total + n },
                    Task::Log(msg) => { print(msg) },
                    Task::Halt => { break }
                }
            }
            total
        }
        let result = run_tasks([
            Task::Log("starting"),
            Task::Compute(10),
            Task::Compute(20),
            Task::Log("halfway"),
            Task::Compute(12),
            Task::Halt,
            Task::Compute(999)
        ])
        print(result)
    "#,
    )
    .expect_output("starting\nhalfway\n42");
}
