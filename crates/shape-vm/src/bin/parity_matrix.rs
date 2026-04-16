//! Parity Matrix CLI
//!
//! Runs all feature tests across Interpreter, VM, and JIT backends
//! to verify execution parity.

use clap::Parser;
use shape_value::ValueWordExt;
use shape_vm::feature_tests::{ParityReport, ParityRunner, all_feature_tests};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "parity_matrix")]
#[command(about = "Test execution parity across Interpreter, VM, and JIT backends")]
struct Args {
    /// Output format: text, json, or markdown
    #[arg(short, long, default_value = "text")]
    format: String,

    /// Only run tests matching this pattern
    #[arg(short, long)]
    filter: Option<String>,

    /// Show verbose output including passing tests
    #[arg(short, long)]
    verbose: bool,

    /// Exit with error code on any failure
    #[arg(long)]
    strict: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    // Get all feature tests
    let mut tests = all_feature_tests();

    // Apply filter if specified
    if let Some(ref filter) = args.filter {
        tests.retain(|t| t.name.contains(filter.as_str()));
    }

    if tests.is_empty() {
        eprintln!("No tests match the filter");
        return ExitCode::FAILURE;
    }

    println!("Running {} parity tests...\n", tests.len());

    // Create runner with default backends
    let runner = ParityRunner::with_defaults();

    // Run all tests
    let report = runner.run_all(&tests);

    // Output results
    match args.format.as_str() {
        "json" => println!("{}", report.format_json()),
        "markdown" => println!("{}", format_markdown(&report)),
        _ => println!("{}", report.format_text()),
    }

    // Determine exit code
    if args.strict && !report.all_passed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn format_markdown(report: &ParityReport) -> String {
    let mut output = String::new();

    output.push_str("# Parity Test Report\n\n");
    output.push_str("## Summary\n\n");
    output.push_str(&format!("| Metric | Count |\n"));
    output.push_str(&format!("|--------|-------|\n"));
    output.push_str(&format!("| Total | {} |\n", report.total));
    output.push_str(&format!("| Passed | {} |\n", report.passed));
    output.push_str(&format!("| Partial | {} |\n", report.partial));
    output.push_str(&format!("| Failed | {} |\n", report.failed));
    output.push_str("\n");

    if report.failed > 0 {
        output.push_str("## Failures\n\n");
        for result in report.failures() {
            output.push_str(&format!("### {}\n\n", result.test_name));
            output.push_str(&format!("```\n{}\n```\n\n", result.format_diff()));
        }
    }

    if report.all_passed() {
        output.push_str("## Result: ✅ ALL TESTS PASSED\n");
    } else {
        output.push_str(&format!("## Result: ❌ {} FAILURES\n", report.failed));
    }

    output
}
