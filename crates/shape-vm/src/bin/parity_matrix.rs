//! Legacy Feature Parity Matrix CLI
//!
//! Runs all feature tests through the in-process legacy feature-test matrix.
//! Real VM-vs-JIT parity is enforced by `scripts/differential-gate.sh`.

use clap::Parser;
use shape_vm::feature_tests::{
    ParityReport, ParityRunner, REAL_JIT_PARITY_CI, REAL_JIT_PARITY_GATE, all_feature_tests,
};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "parity_matrix")]
#[command(
    about = "Run the legacy in-process feature matrix; JIT parity lives in the differential gate"
)]
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

    println!("Running {} legacy feature-matrix tests...", tests.len());
    println!(
        "JIT is not executed by this binary; use {} (wired in {}) for real VM-vs-JIT parity.\n",
        REAL_JIT_PARITY_GATE, REAL_JIT_PARITY_CI
    );

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
    output.push_str(
        "This is the legacy in-process feature matrix. The JIT lane is intentionally skipped; ",
    );
    output.push_str(&format!(
        "real VM-vs-JIT parity is enforced by `{}` via `{}`.\n\n",
        REAL_JIT_PARITY_GATE, REAL_JIT_PARITY_CI
    ));
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

    if report.all_passed() && report.partial == 0 {
        output.push_str("## Result: ✅ ALL TESTS PASSED\n");
    } else if report.all_passed() {
        output.push_str("## Result: NO MISMATCHES (PARTIAL COVERAGE)\n");
    } else {
        output.push_str(&format!("## Result: ❌ {} FAILURES\n", report.failed));
    }

    output
}
