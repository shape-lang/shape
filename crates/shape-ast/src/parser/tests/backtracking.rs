//! WF-0C regression tests: parser cost on deeply nested expressions.
//!
//! Pre-fix, the grammar re-parsed expression prefixes on ordered-choice
//! alternatives (`assignment_expr`, `assignment_expr_no_range`, `range_expr`,
//! `array_literal`), giving ~3.5-4x parse time per nesting level: 12 nested
//! parens or calls took >120s to `shape check` — a front-end DoS on
//! legitimate code. The left-factored rules parse each prefix exactly once.
//!
//! These tests parse depth-16 nested inputs under a hard timeout so a
//! reintroduced backtracking multiplier fails the suite fast instead of
//! hanging it. At depth 16 the pre-fix parser needed hours; the fixed parser
//! needs single-digit milliseconds, so the 10s budget has ample headroom for
//! slow CI while still being decisive.

use super::super::parse_program;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const NESTING_DEPTH: usize = 16;
const PARSE_TIMEOUT: Duration = Duration::from_secs(10);
// Debug-build pest frames are large and recursive descent goes through many
// rule levels per nesting level; the 2MB `thread::spawn` default overflows.
const WORKER_STACK_SIZE: usize = 32 * 1024 * 1024;

/// Parse `input` on a worker thread; fail if it does not complete within
/// `PARSE_TIMEOUT`. A runaway pest parse cannot be interrupted, so on timeout
/// the worker thread is abandoned (detached) — the panic is what matters.
fn assert_parses_within_timeout(input: String, what: &str) {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name(format!("wf0c-parse-{}", what.replace(' ', "-")))
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || {
            let result = parse_program(&input).map(|_| ()).map_err(|e| e.to_string());
            // Send fails only if the receiver already timed out and hung up.
            let _ = tx.send(result);
        })
        .expect("failed to spawn parse worker thread");
    match rx.recv_timeout(PARSE_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("{what} failed to parse: {e}"),
        Err(_) => panic!(
            "{what} did not parse within {PARSE_TIMEOUT:?} — \
             exponential parser backtracking has regressed (WF-0C)"
        ),
    }
}

fn nested(open: &str, close: &str, depth: usize, core: &str) -> String {
    format!(
        "let x = {}{}{};",
        open.repeat(depth),
        core,
        close.repeat(depth)
    )
}

#[test]
fn deeply_nested_parens_parse_quickly() {
    assert_parses_within_timeout(
        nested("(", ")", NESTING_DEPTH, "1"),
        "depth-16 nested parenthesized expression",
    );
}

#[test]
fn deeply_nested_calls_parse_quickly() {
    assert_parses_within_timeout(
        nested("f(", ")", NESTING_DEPTH, "1"),
        "depth-16 nested call expression",
    );
}

#[test]
fn deeply_nested_arrays_parse_quickly() {
    assert_parses_within_timeout(
        nested("[", "]", NESTING_DEPTH, "1"),
        "depth-16 nested array literal",
    );
}

#[test]
fn deeply_nested_mixed_parens_and_arrays_parse_quickly() {
    // Alternating paren/array nesting exercises both left-factored
    // expression rules and the reordered array_literal in one input.
    assert_parses_within_timeout(
        nested("([", "])", NESTING_DEPTH / 2, "1"),
        "depth-16 alternating paren/array nesting",
    );
}
