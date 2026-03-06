//! Integration tests for DateTime builtins via Shape source code.
//!
//! DateTime constructors (DateTime.now, DateTime.utc, DateTime.parse,
//! DateTime.from_epoch) are compiled as VM builtins. The semantic analyzer
//! does not currently recognize `DateTime` as a global, so we bypass it
//! and use the compiler + VM directly (same approach as shape-vm executor
//! tests).

use shape_ast::parser::parse_program;
use shape_value::ValueWord;
use shape_vm::VMConfig;
use shape_vm::compiler::BytecodeCompiler;
use shape_vm::executor::VirtualMachine;

/// Compile and execute Shape source code, returning the final value.
fn eval(source: &str) -> ValueWord {
    let program = parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler.compile(&program).expect("compile failed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed").clone()
}

// ===== Constructors =====

#[test]
fn datetime_now_returns_valid_year() {
    let result = eval(
        r#"
        let dt = DateTime.now()
        dt.year()
    "#,
    );
    let year = result.as_number_coerce().expect("year should be a number");
    assert!(year >= 2024.0, "year should be >= 2024, got {}", year);
}

#[test]
fn datetime_utc_returns_valid_year() {
    let result = eval(
        r#"
        let dt = DateTime.utc()
        dt.year()
    "#,
    );
    let year = result.as_number_coerce().expect("year should be a number");
    assert!(year >= 2024.0, "year should be >= 2024, got {}", year);
}

#[test]
fn datetime_parse_iso8601() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T10:30:00Z")
        dt.year()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(2024.0));
}

#[test]
fn datetime_parse_date_only() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-06-15")
        dt.month()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn datetime_from_epoch_millis() {
    let result = eval(
        r#"
        let dt = DateTime.from_epoch(1705312200000)
        dt.year()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(2024.0));
}

// ===== Component access =====

#[test]
fn datetime_day_extraction() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.day()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn datetime_hour_extraction() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.hour()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(14.0));
}

#[test]
fn datetime_minute_second_extraction() {
    let minute = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.minute()
    "#,
    );
    assert_eq!(minute.as_number_coerce(), Some(30.0));

    let second = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.second()
    "#,
    );
    assert_eq!(second.as_number_coerce(), Some(45.0));
}

#[test]
fn datetime_day_of_week() {
    // 2024-01-01 is Monday (0-indexed from Monday)
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-01T00:00:00Z")
        dt.day_of_week()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(0.0));
}

#[test]
fn datetime_day_of_year() {
    // Feb 1 is day 32 (Jan has 31 days)
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-02-01T00:00:00Z")
        dt.day_of_year()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(32.0));
}

#[test]
fn datetime_is_weekday() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-01T00:00:00Z")
        dt.is_weekday()
    "#,
    );
    assert!(result.is_truthy(), "Monday should be a weekday");
}

#[test]
fn datetime_is_weekend() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-06T00:00:00Z")
        dt.is_weekend()
    "#,
    );
    assert!(result.is_truthy(), "Saturday should be a weekend day");
}

// ===== Arithmetic =====

#[test]
fn datetime_add_days() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T00:00:00Z")
        let later = dt.add_days(10)
        later.day()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(25.0));
}

#[test]
fn datetime_add_hours() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T10:00:00Z")
        let later = dt.add_hours(5)
        later.hour()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn datetime_add_days_crosses_month() {
    let month = eval(
        r#"
        let dt = DateTime.parse("2024-01-30T12:00:00Z")
        let later = dt.add_days(2)
        later.month()
    "#,
    );
    assert_eq!(month.as_number_coerce(), Some(2.0));

    let day = eval(
        r#"
        let dt = DateTime.parse("2024-01-30T12:00:00Z")
        let later = dt.add_days(2)
        later.day()
    "#,
    );
    assert_eq!(day.as_number_coerce(), Some(1.0));
}

#[test]
fn datetime_add_hours_crosses_day() {
    let day = eval(
        r#"
        let dt = DateTime.parse("2024-01-01T22:00:00Z")
        let later = dt.add_hours(5)
        later.day()
    "#,
    );
    assert_eq!(day.as_number_coerce(), Some(2.0));

    let hour = eval(
        r#"
        let dt = DateTime.parse("2024-01-01T22:00:00Z")
        let later = dt.add_hours(5)
        later.hour()
    "#,
    );
    assert_eq!(hour.as_number_coerce(), Some(3.0));
}

// ===== Comparison =====

#[test]
fn datetime_is_before() {
    let result = eval(
        r#"
        let a = DateTime.parse("2024-01-01T00:00:00Z")
        let b = DateTime.parse("2024-06-01T00:00:00Z")
        a.is_before(b)
    "#,
    );
    assert!(result.is_truthy(), "Jan 1 should be before Jun 1");
}

#[test]
fn datetime_is_after() {
    let result = eval(
        r#"
        let a = DateTime.parse("2024-06-01T00:00:00Z")
        let b = DateTime.parse("2024-01-01T00:00:00Z")
        a.is_after(b)
    "#,
    );
    assert!(result.is_truthy(), "Jun 1 should be after Jan 1");
}

#[test]
fn datetime_is_same_day() {
    let result = eval(
        r#"
        let a = DateTime.parse("2024-03-15T08:00:00Z")
        let b = DateTime.parse("2024-03-15T22:30:00Z")
        a.is_same_day(b)
    "#,
    );
    assert!(
        result.is_truthy(),
        "same date different time should be same day"
    );
}

// ===== Formatting =====

#[test]
fn datetime_iso8601_output() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:00Z")
        dt.iso8601()
    "#,
    );
    let formatted = result.as_str().expect("iso8601() should return a string");
    assert!(
        formatted.starts_with("2024-03-15"),
        "iso8601 should start with '2024-03-15', got '{}'",
        formatted
    );
}

#[test]
fn datetime_unix_timestamp() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T10:30:00Z")
        dt.unix_timestamp()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(1_705_314_600.0));
}

// ===== Timezone =====

#[test]
fn datetime_utc_timezone_string() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-01T00:00:00Z")
        dt.timezone()
    "#,
    );
    assert_eq!(result.as_str(), Some("UTC"));
}

#[test]
fn datetime_to_utc_normalizes_offset() {
    let hour = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T15:30:00+05:00")
        let utc = dt.to_utc()
        utc.hour()
    "#,
    );
    assert_eq!(hour.as_number_coerce(), Some(10.0));

    let tz = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T15:30:00+05:00")
        let utc = dt.to_utc()
        utc.timezone()
    "#,
    );
    assert_eq!(tz.as_str(), Some("UTC"));
}
