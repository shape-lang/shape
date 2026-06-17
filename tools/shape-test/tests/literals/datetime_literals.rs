//! `@"..."` DateTime literal tests (STAGE DT1).
//!
//! The book chapter `fundamentals/datetime.mdx` §"DateTime Literals" documents
//! `@"<iso8601>"` as source-level DateTime literal syntax that produces a
//! `DateTime` value on which the 30 seeded instance methods resolve.
//!
//! Two bugs blocked this before STAGE DT1:
//!   1. Grammar: `datetime_literal = string ~ timezone?` had a bare trailing
//!      `ident` timezone that — because whitespace/newlines are skipped —
//!      greedily ate the identifier on the *next* statement line, so
//!      `let d = @"2024-01-15"\nprint(d)` parsed as `@"2024-01-15" print` then
//!      a `(d)` call and the `let` binding never took ("Undefined variable
//!      'd'"). The timezone is always carried *inside* the ISO string, and the
//!      AST builder never read the trailing ident, so it was dropped.
//!   2. Inference: `Expr::DateTime` inferred `Basic("datetime")`, but the
//!      strict `MethodTable` registers the 30 DateTime methods under the key
//!      `"DateTime"`, so `d.year()` reported "Method 'year' not found on type
//!      'datetime'". Inference now yields `Reference("DateTime")`.
//!
//! All datetimes here are FIXED epochs (no `now()`), so output is deterministic.

use shape_test::shape_test::ShapeTest;

#[test]
fn datetime_literal_let_year() {
    ShapeTest::new(
        r#"
        let d = @"2026-01-15T10:30:00Z"
        print(d.year())
    "#,
    )
    .expect_output("2026");
}

#[test]
fn datetime_literal_components() {
    // Fixed datetime; assert each component method against the known value.
    ShapeTest::new(
        r#"
        let d = @"2024-06-15T14:30:45+00:00"
        print(d.year())
        print(d.month())
        print(d.day())
        print(d.hour())
        print(d.minute())
        print(d.second())
    "#,
    )
    .expect_output("2024\n6\n15\n14\n30\n45");
}

#[test]
fn datetime_literal_format() {
    ShapeTest::new(
        r#"
        let d = @"2026-01-15T10:30:00Z"
        print(d.format("%Y-%m-%d"))
    "#,
    )
    .expect_output("2026-01-15");
}

#[test]
fn datetime_literal_add_days_then_day() {
    ShapeTest::new(
        r#"
        let d = @"2026-01-15T10:30:00Z"
        print(d.add_days(1).day())
    "#,
    )
    .expect_output("16");
}

#[test]
fn datetime_literal_date_only_midnight() {
    // Date-only literal defaults to midnight UTC per the book §Parsing Strings.
    ShapeTest::new(
        r#"
        let d = @"2024-01-15"
        print(d.hour())
        print(d.minute())
        print(d.unix_timestamp())
    "#,
    )
    .expect_output("0\n0\n1705276800");
}

#[test]
fn datetime_literal_iso8601_roundtrip() {
    ShapeTest::new(
        r#"
        let d = @"2024-06-15T14:30:00+00:00"
        print(d.iso8601())
    "#,
    )
    .expect_output("2024-06-15T14:30:00+00:00");
}

#[test]
fn datetime_literal_is_weekend() {
    // 2024-01-06 is a Saturday (book §Day Information example).
    ShapeTest::new(
        r#"
        let d = @"2024-01-06T12:00:00+00:00"
        print(d.is_weekend())
    "#,
    )
    .expect_output("true");
}

#[test]
fn datetime_literal_as_fn_arg() {
    // A literal bound in a `let` then passed as a function argument; the method
    // resolves inside the callee body.
    ShapeTest::new(
        r#"
        fn report_year(d) {
            print(d.year())
        }
        let d = @"2026-01-15T10:30:00Z"
        report_year(d)
    "#,
    )
    .expect_output("2026");
}
