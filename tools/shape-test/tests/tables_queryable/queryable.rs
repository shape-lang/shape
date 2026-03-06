//! Tests for LINQ-style from..in..where..select query syntax.
//!
//! This syntax desugars to method chains:
//!   from x in arr where x > 1 select x * 2
//!   => arr.filter(|x| x > 1).map(|x| x * 2)

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic from..in..select
// =========================================================================

#[test]
fn from_select_identity() {
    // Simplest query: just select each element
    ShapeTest::new(
        r#"
        let result = from x in [10, 20, 30] select x
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("10\n20\n30");
}

#[test]
fn from_select_transform() {
    // Select with transformation
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] select x * 2
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n6");
}

// =========================================================================
// from..in..where..select
// =========================================================================

#[test]
fn from_where_select() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5] where x > 3 select x
        print(result.length)
        print(result[0])
        print(result[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n5");
}

#[test]
fn from_where_select_with_transform() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5] where x > 2 select x * 10
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("30\n40\n50");
}

// =========================================================================
// from..in with let clause
// =========================================================================

#[test]
// TDD: Semantic analyzer does not track variables introduced by let-clauses in from-queries
fn from_let_select() {
    // let clause introduces a computed variable
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] let doubled = x * 2 select doubled
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n6");
}

// =========================================================================
// Queryable trait (type system level)
// =========================================================================

#[test]
// TDD: Queryable trait impl blocks cannot bind unbound type variables in semantic analyzer
fn queryable_trait_on_custom_type() {
    // The Queryable<T> trait exists in the type system but impl blocks for it
    // on custom types are not yet fully supported at runtime.
    ShapeTest::new(
        r#"
        trait Queryable<T> {
            fn filter(self, pred: fn(T) -> bool) -> Self
            fn map<U>(self, f: fn(T) -> U) -> Array<U>
        }
        type MyList {
            items: Array<int>
        }
        impl Queryable for MyList {
            fn filter(self, pred) {
                MyList { items: self.items.filter(pred) }
            }
            fn map(self, f) {
                self.items.map(f)
            }
        }
        let list = MyList { items: [1, 2, 3] }
        let result = list.filter(|x| x > 1)
        print(result.items.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}
