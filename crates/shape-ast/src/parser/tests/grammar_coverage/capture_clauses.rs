//! ADR-009 C1 declared-capture-clause grammar proofs.

use super::*;

/// Pull the sole closure's clause out of a program. Serde keeps this helper
/// independent of the surrounding statement/item shape.
fn sole_capture_clause(input: &str) -> Option<crate::ast::CaptureClause> {
    let program = crate::parse_program(input).expect("fixture parses");
    let json = serde_json::to_value(&program).expect("AST serializes");

    fn find(value: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(function) = map.get("FunctionExpr")
                    && let Some(captures) = function.get("captures")
                {
                    out.push(captures.clone());
                }
                for nested in map.values() {
                    find(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    find(nested, out);
                }
            }
            _ => {}
        }
    }

    let mut found = Vec::new();
    find(&json, &mut found);
    assert_eq!(found.len(), 1, "fixture must contain exactly one closure");
    serde_json::from_value(found.remove(0)).expect("the carrier round-trips")
}

/// Test the pipe-lambda rule directly because statement recovery can accept a
/// malformed enclosing program. A partial match is not acceptance.
fn pipe_lambda_parses(literal: &str) -> bool {
    ShapeParser::parse(Rule::pipe_lambda, literal)
        .map(|mut pairs| pairs.next().is_some_and(|pair| pair.as_str() == literal))
        .unwrap_or(false)
}

#[test]
fn capture_clause_parses_move_and_share() {
    let source = r#"let f = |acc, item; move cfg, share total| acc + item"#;
    let clause = sole_capture_clause(source).expect("the clause is carried on the FunctionExpr");
    assert_eq!(clause.len(), 2);
    assert_eq!(clause.entries[0].mode, crate::ast::CaptureMode::Move);
    assert_eq!(clause.entries[0].name, "cfg");
    assert_eq!(
        &source[clause.entries[0].span.start..clause.entries[0].span.end],
        "move cfg"
    );
    assert_eq!(
        &source[clause.entries[0].name_span.start..clause.entries[0].name_span.end],
        "cfg"
    );
    assert_eq!(clause.entries[1].mode, crate::ast::CaptureMode::Share);
    assert_eq!(clause.entries[1].name, "total");
    assert_eq!(
        &source[clause.entries[1].span.start..clause.entries[1].span.end],
        "share total"
    );
    assert_eq!(
        &source[clause.entries[1].name_span.start..clause.entries[1].name_span.end],
        "total"
    );
}

#[test]
fn capture_clause_parses_with_no_params() {
    let clause = sole_capture_clause(r#"let f = |; move handle| handle"#).expect("clause");
    assert_eq!(clause.len(), 1);
    assert_eq!(clause.entries[0].mode, crate::ast::CaptureMode::Move);
    assert_eq!(clause.entries[0].name, "handle");
}

/// An empty clause declares that the closure captures nothing; no clause means
/// inference (or the generated-code missing-declaration rejection).
#[test]
fn empty_capture_clause_is_some_not_none() {
    let clause = sole_capture_clause(r#"let f = |x;| x"#).expect("an empty clause still parses");
    assert!(clause.is_empty());
}

#[test]
fn no_clause_leaves_the_carrier_none() {
    assert!(
        sole_capture_clause(r#"let f = |x| x + 1"#).is_none(),
        "a closure with no clause carries None — the compiler then infers"
    );
}

/// Borrow spellings parse so the compiler can issue the named region error.
#[test]
fn capture_clause_parses_borrow_spellings() {
    let clause = sole_capture_clause(r#"let f = |x; &a, &mut b| x"#).expect("clause");
    assert_eq!(
        clause.entries[0].mode,
        crate::ast::CaptureMode::SharedBorrow
    );
    assert_eq!(clause.entries[0].name, "a");
    assert_eq!(
        clause.entries[1].mode,
        crate::ast::CaptureMode::ExclusiveBorrow
    );
    assert_eq!(clause.entries[1].name, "b");
}

/// A capture is a binding reference, never a string-selected name.
#[test]
fn string_form_capture_does_not_parse() {
    assert!(pipe_lambda_parses(r#"|x; move cfg| x"#));
    assert!(!pipe_lambda_parses(r#"|x; move "cfg"| x"#));
    assert!(!pipe_lambda_parses(r#"|x; capture("cfg")| x"#));
}

#[test]
fn capture_entry_without_a_mode_does_not_parse() {
    assert!(!pipe_lambda_parses(r#"|x; cfg| x"#));
}

/// `share` is contextual rather than globally reserved.
#[test]
fn share_is_not_a_reserved_word() {
    crate::parse_program(
        r#"let share = 2
let doubled = share + share
"#,
    )
    .expect("`share` stays an ordinary identifier outside a capture clause");
}
