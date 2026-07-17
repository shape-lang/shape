//! `replace body` provenance preservation through the `ctx.original` rewrite.

use super::*;
use shape_ast::transform::stamp_generated_closures;

fn stamped_body(src: &str) -> Vec<Statement> {
    let program = shape_ast::parser::parse_program(src).expect("parse");
    let mut body = program
        .items
        .into_iter()
        .find_map(|item| match item {
            shape_ast::ast::Item::Function(f, _) => Some(f.body),
            _ => None,
        })
        .expect("function");
    // K1b: the stamp comes from the one mint in expansion_provenance.rs.
    let origin =
        crate::compiler::comptime_builtins::expansion_provenance::GeneratedOrigin::node_origin_for_tests(
            &["fn:compute", "replace_body"],
            "compute",
        );
    stamp_generated_closures(&mut body, &origin);
    body
}

fn closure_paths(stmts: &[Statement]) -> Vec<Vec<String>> {
    let json = serde_json::to_value(stmts).unwrap();
    let mut out = Vec::new();
    fn visit(v: &serde_json::Value, out: &mut Vec<Vec<String>>) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(f) = map.get("FunctionExpr")
                    && let Some(path) = f.pointer("/generated_origin/node_path")
                {
                    out.push(
                        path.as_array()
                            .unwrap()
                            .iter()
                            .map(|s| s.as_str().unwrap().to_string())
                            .collect(),
                    );
                }
                for value in map.values() {
                    visit(value, out);
                }
            }
            serde_json::Value::Array(items) => {
                for value in items {
                    visit(value, out);
                }
            }
            _ => {}
        }
    }
    visit(&json, &mut out);
    out
}

#[test]
fn stamp_survives_the_ctx_original_rewrite_including_nested_closures() {
    // The rewrite rebuilds every node it walks, so this exercises the rebuild
    // path rather than a clone.
    let body = stamped_body(
        "fn f() -> int { let outer = || { let inner = || ctx.original() + 1; inner() }; outer() }",
    );
    let before = closure_paths(&body);
    assert_eq!(
        before.len(),
        2,
        "fixture must have an outer + nested closure"
    );

    let bound: HashSet<String> = HashSet::new();
    let rewritten = rewrite_original_calls_in_statements(&body, &bound, "__shadow__");

    assert!(
        contains_shadow_call(&rewritten, "__shadow__"),
        "fixture must actually exercise the rewrite"
    );
    assert_eq!(
        closure_paths(&rewritten),
        before,
        "the rewrite dropped the generated-node stamp — the `replace body` \
         capture gate would go blind"
    );
}

fn contains_shadow_call(stmts: &[Statement], shadow: &str) -> bool {
    serde_json::to_string(stmts).unwrap().contains(shadow)
}
