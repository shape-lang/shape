//! Integration tests for DateTime builtins and methods.
//!
//! Tests compile and run Shape source code to verify:
//! - DateTime.now(), DateTime.utc(), DateTime.parse(), DateTime.from_epoch()
//! - DateTime component extraction (year, month, day, hour, etc.)
//! - DateTime arithmetic (add_days, add_hours)
//! - DateTime comparison (is_before, is_after, is_same_day)
//! - DateTime formatting
//! - Timezone conversion

use crate::VMConfig;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

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

#[test]
fn test_datetime_now_returns_datetime() {
    // DateTime.now() should return a DateTime value with a valid year
    let result = eval(
        r#"
        let dt = DateTime.now()
        dt.year()
    "#,
    );
    let year = result.as_number_coerce().expect("year should be a number");
    assert!(
        year >= 2024.0,
        "DateTime.now().year() should be >= 2024, got {}",
        year
    );
}

#[test]
fn test_datetime_utc_returns_utc() {
    // DateTime.utc() should return a UTC DateTime with a valid year
    let result = eval(
        r#"
        let dt = DateTime.utc()
        dt.year()
    "#,
    );
    let year = result.as_number_coerce().expect("year should be a number");
    assert!(
        year >= 2024.0,
        "DateTime.utc().year() should be >= 2024, got {}",
        year
    );
}

#[test]
fn test_datetime_parse_iso8601() {
    // DateTime.parse() should parse ISO 8601 format
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T10:30:00Z")
        dt.year()
    "#,
    );
    assert_eq!(
        result.as_number_coerce(),
        Some(2024.0),
        "parsed DateTime year should be 2024"
    );
}

#[test]
fn test_datetime_parse_date_only() {
    // DateTime.parse() should handle date-only format
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-06-15")
        dt.month()
    "#,
    );
    assert_eq!(
        result.as_number_coerce(),
        Some(6.0),
        "parsed DateTime month should be 6"
    );
}

#[test]
fn test_datetime_from_epoch() {
    // DateTime.from_epoch() should create DateTime from milliseconds
    let result = eval(
        r#"
        let dt = DateTime.from_epoch(1705312200000)
        dt.year()
    "#,
    );
    assert_eq!(
        result.as_number_coerce(),
        Some(2024.0),
        "from_epoch DateTime year should be 2024"
    );
}

#[test]
fn test_datetime_components() {
    // Year, month, day, hour extraction
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.day()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(15.0), "day should be 15");
}

#[test]
fn test_datetime_hour_extraction() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-03-15T14:30:45Z")
        dt.hour()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(14.0), "hour should be 14");
}

#[test]
fn test_datetime_arithmetic_add_days() {
    // add_days should produce a new DateTime
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T00:00:00Z")
        let later = dt.add_days(10)
        later.day()
    "#,
    );
    assert_eq!(
        result.as_number_coerce(),
        Some(25.0),
        "Jan 15 + 10 days = Jan 25"
    );
}

#[test]
fn test_datetime_arithmetic_add_hours() {
    let result = eval(
        r#"
        let dt = DateTime.parse("2024-01-15T10:00:00Z")
        let later = dt.add_hours(5)
        later.hour()
    "#,
    );
    assert_eq!(
        result.as_number_coerce(),
        Some(15.0),
        "10:00 + 5 hours = 15:00"
    );
}

#[test]
fn test_datetime_comparison_is_before() {
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
fn test_datetime_comparison_is_after() {
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
fn test_datetime_iso8601_output() {
    // iso8601() should produce a formatted ISO string
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
