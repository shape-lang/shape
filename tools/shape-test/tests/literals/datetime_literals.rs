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

// ===== Operator arithmetic (STAGE DT2) =====
//
// The datetime book chapter (fundamentals/datetime §Operator Arithmetic)
// documents `+`/`-` on DateTime/Duration values:
//   DateTime + Duration -> DateTime
//   DateTime - Duration -> DateTime
//   DateTime - DateTime -> Duration
//   Duration ± Duration -> Duration
//
// These must TYPE-CHECK under strict typing. Before STAGE DT2 they failed
// the type-checker: a `let a = DateTime.parse(..)` binding lowered to
// `unknown` (so `a - b` reported "operand types are `unknown` and `unknown`")
// and a Duration operand was rejected as non-Numeric ("`duration` does not
// implement trait `Numeric`"). The fix recognizes the DateTime/Duration
// operand types and produces the documented result type WITHOUT weakening
// `int != number` — Duration is its own type, not Numeric, so there is no
// silent coercion. All datetimes are FIXED epochs (no `now()`).

#[test]
fn datetime_plus_duration_yields_datetime() {
    // DateTime + Duration -> DateTime (book: `a + 3d` => day 18).
    ShapeTest::new(
        r#"
        let a = DateTime.parse("2024-06-15T12:00:00+00:00")
        let future = a + 3d
        print(future.day())
    "#,
    )
    .expect_output("18");
}

#[test]
fn datetime_minus_duration_yields_datetime() {
    // DateTime - Duration -> DateTime (book: `a - 1w` => day 8).
    ShapeTest::new(
        r#"
        let a = DateTime.parse("2024-06-15T12:00:00+00:00")
        let past = a - 1w
        print(past.day())
    "#,
    )
    .expect_output("8");
}

#[test]
fn datetime_minus_datetime_yields_duration() {
    // DateTime - DateTime -> Duration. 2024-06-15 minus 2024-06-10 is 5 days;
    // the Duration renders as the ISO-8601 form (5 days == 432000 seconds).
    ShapeTest::new(
        r#"
        let a = DateTime.parse("2024-06-15T12:00:00+00:00")
        let b = DateTime.parse("2024-06-10T12:00:00+00:00")
        let diff = a - b
        print(f"{diff}")
    "#,
    )
    .expect_output("PT432000S");
}

#[test]
fn datetime_literal_chained_duration_arithmetic() {
    // Book runnable example (datetime.mdx §DateTime Literals):
    // `@"2024-06-15" + 30d - 1w` == 2024-07-08. Exercises a `@"..."` DateTime
    // literal as the left operand of chained `+`/`-` duration arithmetic.
    ShapeTest::new(
        r#"
        let future = @"2024-06-15" + 30d - 1w
        print(future.format("%Y-%m-%d"))
    "#,
    )
    .expect_output("2024-07-08");
}

#[test]
fn duration_not_numeric_rejects_plus_int() {
    // Strict guard: Duration is NOT Numeric. `1d + 1` (Duration + int) is a
    // compile error — the temporal operator rules only accept the documented
    // DateTime/Duration operand combinations; there is no silent coercion of a
    // Duration to a number.
    ShapeTest::new(
        r#"
        let bad = 1d + 1
        print(bad)
    "#,
    )
    .expect_run_err_contains("Numeric");
}

#[test]
fn datetime_format_tz_name_z_specifier() {
    // Book datetime.mdx §Formatting: `%Z` renders the timezone *name* (`UTC`
    // for a UTC datetime), NOT the numeric offset chrono gives a FixedOffset.
    // `%z` stays the numeric offset. Multi-timezone report shape from the book.
    ShapeTest::new(
        r#"
        let dt = DateTime.parse("2024-06-15T14:30:00+00:00")
        print(dt.format("%H:%M %Z"))
        print(dt.format("%z"))
    "#,
    )
    .expect_output("14:30 UTC\n+0000");
}

#[test]
fn datetime_method_result_array_element_infers() {
    // STAGE DT4 (concrete_type_for_expr DateTime-instance-method arm): a
    // DateTime instance method on a proven-DateTime receiver surfaces its
    // documented return `ConcreteType`, so an array literal whose element is
    // such a call proves a homogeneous element kind. Pre-fix,
    // `[dt.format(..)]` / `[dt.year()]` surfaced "cannot infer the element
    // type of this array literal" because the method result was opaque to the
    // bytecode compiler's element-type resolver.
    ShapeTest::new(
        r#"
        let dt = @"2024-06-15T14:30:45+00:00"
        let strs = [dt.format("%Y-%m-%d")]
        let ints = [dt.year()]
        print(strs[0])
        print(ints[0])
    "#,
    )
    .expect_output("2024-06-15\n2024");
}

#[test]
fn datetime_array_accumulation_with_annotation() {
    // Book datetime.mdx §Date Range Iteration shape: accumulate formatted
    // DateTime strings into an annotated `Array<string>` across an add_days
    // loop. Exercises the DT4 method-result element-type proof end to end on
    // an `acc + [dt.format(..)]` reassignment. FIXED epochs — deterministic.
    ShapeTest::new(
        r#"
        let start = @"2024-01-01T00:00:00+00:00"
        let stop = @"2024-01-04T00:00:00+00:00"
        let mut current = start
        let mut days: Array<string> = []
        while current.is_before(stop) or current.is_same_day(stop) {
            days = days + [current.format("%Y-%m-%d")]
            current = current.add_days(1)
        }
        print(days.length())
        for d in days { print(d) }
    "#,
    )
    .expect_output("4\n2024-01-01\n2024-01-02\n2024-01-03\n2024-01-04");
}
