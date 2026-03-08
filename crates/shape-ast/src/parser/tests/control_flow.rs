//! Control flow parsing tests
//!
//! This module contains tests for control flow constructs:
//! - For loops with range expressions
//! - Match expressions with patterns
//! - Block expressions with return statements
//! - Flow control in annotation metadata

use super::super::*;
use crate::error::{Result, ShapeError};

/// Helper to parse a full program
fn parse_program_helper(input: &str) -> Result<Vec<crate::ast::Item>> {
    let pairs = ShapeParser::parse(Rule::program, input).map_err(|e| ShapeError::ParseError {
        message: e.to_string(),
        location: None,
    })?;

    let mut items = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if let Rule::item = inner.as_rule() {
                    items.push(parse_item(inner)?);
                }
            }
        }
    }
    Ok(items)
}

// =========================================================================
// For Loop Tests
// =========================================================================

#[test]
fn test_parse_range_in_for() {
    // Range expression in for loop
    let content = "for i in 0..10 { print(i); }";
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Range in for loop should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Match Expression Tests
// =========================================================================

#[test]
fn test_enum_match_pattern_qualified() {
    let content = r#"
        let x = match signal {
            Signal::Buy => 1,
            Signal::Limit { price: p } => p
        };
    "#;
    let items = parse_program_helper(content).expect("enum match should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 2);
            if let crate::ast::Pattern::Constructor {
                enum_name, variant, ..
            } = &match_expr.arms[0].pattern
            {
                assert_eq!(enum_name.as_deref(), Some("Signal"));
                assert_eq!(variant, "Buy");
            } else {
                panic!("Expected constructor pattern");
            }
        } else {
            panic!("Expected match expression, got {:?}", value);
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_enum_with_typed_function_param() {
    let result = parse_program_helper(
        r#"
        enum Status { Active, Inactive, Pending }

        function check(s: Status) {
            return match s {
                Status::Active => "yes"
            };
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "Enum with typed function param should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_match_arms_without_commas() {
    let content = r#"
        let x = match signal {
            Signal::Buy => 1
            Signal::Sell => -1
            _ => 0
        };
    "#;
    let items = parse_program_helper(content).expect("match without commas should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 3);
        } else {
            panic!("Expected match expression, got {:?}", value);
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_match_arm_with_block_body_no_comma() {
    // The exact user scenario: block body on first arm, no comma before next arm
    let content = r#"
        match snapshot() {
            Snapshot::Hash(id) => {
                print("Paused at: " + id)
                exit(0)
            }
            Snapshot::Resumed => print("Back in action!")
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Match with block body and no comma should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_match_function_call_scrutinee_with_block_arms_and_commas() {
    let content = r#"
        match snapshot() {
            Snapshot::Hash(id) => {
                print("Paused at: " + id)
                exit(0)
            },
            Snapshot::Resumed => {
                print("Back in action!")
            }
        }
    "#;

    let items = parse_program_helper(content).expect("match snapshot() should parse");
    assert_eq!(items.len(), 1, "expected one top-level statement");

    let match_expr = match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::Expression(expr, _), _) => match expr {
            crate::ast::Expr::Match(match_expr, _) => match_expr,
            other => panic!("expected top-level match expression, got {:?}", other),
        },
        other => panic!("expected top-level statement, got {:?}", other),
    };

    match match_expr.scrutinee.as_ref() {
        crate::ast::Expr::FunctionCall { name, .. } => {
            assert_eq!(name, "snapshot", "expected snapshot() scrutinee");
        }
        other => panic!("expected function-call scrutinee, got {:?}", other),
    }

    assert_eq!(match_expr.arms.len(), 2, "expected exactly two match arms");
}

#[test]
fn test_match_arms_with_trailing_comma() {
    let content = r#"
        let x = match s {
            "a" => 1,
            "b" => 2,
        };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Match with trailing comma should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_match_typed_pattern_parses() {
    let content = r#"
        let v = match (x) {
            n: int => n,
            s: string => 0
        };
    "#;
    let items = parse_program_helper(content).expect("typed match patterns should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 2);
            match &match_expr.arms[0].pattern {
                crate::ast::Pattern::Typed {
                    name,
                    type_annotation,
                } => {
                    assert_eq!(name, "n");
                    assert_eq!(
                        type_annotation,
                        &crate::ast::TypeAnnotation::Basic("int".to_string())
                    );
                }
                other => panic!("Expected typed pattern, got {:?}", other),
            }
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_empty_match_accepted_by_grammar() {
    // The grammar allows empty match (zero arms) via the optional match_arm list.
    let content = r#"
        fn afunc(c) {
            match c {
            }
            c = c + 1
            return c
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Empty match (zero arms) should be accepted by the grammar: {:?}",
        result.err()
    );
}

#[test]
fn test_empty_match_with_prior_call_accepted() {
    let content = r#"
        fn afunc(c) {
            print("func called with " + c)
            match c {

            }
            c = c + 1
            return c
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Empty match (zero arms) should be accepted even with prior call: {:?}",
        result.err()
    );
}

#[test]
fn test_empty_match_function_accepted() {
    let content = r#"
        fn afunc(c) {
            print("func called with " + c)
            match c {

            }
            c = c + 1
            return c
        }
        let x = 1
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Empty match (zero arms) in function body should be accepted: {:?}",
        result.err()
    );
}

#[test]
fn test_statement_rule_accepts_empty_match() {
    let stmt = "match c {\n\n}";
    let parsed = ShapeParser::parse(Rule::statement, stmt);
    assert!(
        parsed.is_ok(),
        "statement rule should accept empty match expression (zero arms): {:?}",
        parsed.err()
    );
}

#[test]
fn test_typed_match_with_commented_line_parses() {
    let content = r#"
        fn afunc(c) {
            //print("func called with " + c)
            let result = match c {
                c: int => c + 1
                _ => 1
            }
            return c
            return "hi"
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Function with commented line + typed match should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_full_program_with_from_and_typed_match_parses() {
    let content = r#"
from std::core::snapshot use { Snapshot }

let x = {x: 1}
let y = | x | 10*(x.x*2)
print(f"this is {y(x)}")

x.y = 1
let i = 10D

let c = "d"

fn afunc(c) {
  //print("func called with " + c)
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  //c = c + 1
  return c

  return "hi"
}
"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Program with from + typed match should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Property Assignment Tests
// =========================================================================

#[test]
fn test_property_assignment_parse() {
    // Simple property assignment must parse
    let content = "a.y = 2";
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "a.y = 2 should parse as expression_stmt: {:?}",
        result.err()
    );
}

#[test]
fn test_property_assignment_in_context() {
    let content = r#"
        let a = {x: 1}
        a.y = 2
        a.z = "hello"
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Property assignments should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_property_assignment_expr_rule() {
    // Test that assignment_expr can parse "a.y = 2"
    let content = "a.y = 2";
    let pest_result = ShapeParser::parse(Rule::assignment_expr, content);
    assert!(
        pest_result.is_ok(),
        "assignment_expr should parse 'a.y = 2': {:?}",
        pest_result.err()
    );
}

#[test]
fn test_assign_op_rule() {
    // Test that assign_op can parse "="
    let content = "= 2";
    let pest_result = ShapeParser::parse(Rule::assign_op, content);
    assert!(
        pest_result.is_ok(),
        "assign_op should parse '=': {:?}",
        pest_result.err()
    );
}

// =========================================================================
// Block and Return Tests
// =========================================================================

#[test]
fn test_block_expr_without_semicolons() {
    let content = r#"
        let x = {
            let y = 10
            let z = 20
            y + z
        };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Block without semicolons should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_block_expr_mixed_semicolons() {
    // Semicolons only needed for one-line separation
    let content = r#"
        let x = {
            let y = 10; let z = 20
            y + z
        };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Block with mixed semicolons should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_block_expr_with_return() {
    let content = r#"
        let x = {
            let y = 10;
            return y * 2
        };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Block with return should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_annotation_def_with_return_in_metadata() {
    let content = r#"
        annotation cached(ttl) {
            before(fn, args, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.get(key)
            }

            after(fn, args, result, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.set(key, result);
                result
            }

            metadata() {
                return {
                    cacheable: true,
                    ttl: ttl
                }
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with return in metadata should parse: {:?}",
        result.err()
    );
}

// ===== Sprint 5: Async Join + Annotated Expression Tests =====

#[test]
fn test_await_join_all_basic() {
    let content = r#"
        async function fetch_all() {
            let result = await join all {
                fetch("a"),
                fetch("b"),
                fetch("c"),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join all should parse: {:?}",
        result.err()
    );

    // Verify it parsed into at least one item
    let items = result.unwrap();
    assert!(!items.is_empty(), "Should have parsed at least one item");
}

#[test]
fn test_await_join_race() {
    let content = r#"
        async function first_response() {
            await join race {
                fetch("fast"),
                fetch("slow"),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join race should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_join_any() {
    let content = r#"
        async function any_success() {
            await join any {
                try_endpoint_a(),
                try_endpoint_b(),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join any should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_join_settle() {
    let content = r#"
        async function all_settled() {
            await join settle {
                might_fail_a(),
                might_fail_b(),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join settle should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_join_named_branches() {
    let content = r#"
        async function named() {
            await join all {
                prices: fetch_prices(),
                volume: fetch_volume(),
                news: fetch_news(),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join with named branches should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_annotated_expression() {
    let content = r#"
        async function with_timeout() {
            await @timeout(5s) fetch("slow_endpoint")
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await @annotation expr should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_plain_expression() {
    let content = r#"
        async function plain() {
            await fetch("data")
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "plain await should still parse: {:?}",
        result.err()
    );
}

#[test]
fn test_await_join_single_branch() {
    let content = r#"
        async function single() {
            await join all {
                fetch("only_one"),
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "await join with single branch should parse: {:?}",
        result.err()
    );
}

// ===== Comptime Block Tests =====

#[test]
fn test_comptime_block_expression_simple() {
    let content = r#"
        let x = comptime { 2 + 3 }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "comptime block expression should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    // Should be a variable declaration with a comptime expression on the RHS
    assert_eq!(items.len(), 1);
}

#[test]
fn test_comptime_block_with_statements() {
    let content = r#"
        let size = comptime {
            let a = 10
            let b = 20
            a + b
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "comptime block with multiple statements should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_comptime_block_top_level() {
    let content = r#"
        comptime {
            let x = 42
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "top-level comptime block should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Comptime(stmts, _) => {
            assert!(!stmts.is_empty(), "comptime block should have statements");
        }
        other => panic!("expected Item::Comptime, got {:?}", other),
    }
}

#[test]
fn test_comptime_block_as_expression() {
    let content = r#"
        let result = comptime { "hello" }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "comptime block as expression should parse: {:?}",
        result.err()
    );
}

// ===== Comptime For Tests =====

#[test]
fn test_comptime_for_basic() {
    let content = r#"
        comptime for field in target.fields {
            let x = 1
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "comptime for should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_comptime_for_in_expression() {
    let content = r#"
        let result = comptime for f in items {
            print(f.name)
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "comptime for as expression should parse: {:?}",
        result.err()
    );
}

// ===== Annotated Expression Tests =====

#[test]
fn test_annotated_expression_simple() {
    let content = r#"
        @timed compute()
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "@timed expr should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_annotated_expression_with_args() {
    let content = r#"
        @timeout(5000) fetch("data")
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "@timeout(5000) expr should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_annotated_expression_multiple() {
    let content = r#"
        @retry(3) @timeout(5000) fetch("data")
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "multiple annotations should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_annotated_expression_produces_nested_annotated() {
    let content = r#"
        @retry(3) @timeout(5000) fetch("data")
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "multi-annotation should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    // Extract expression from the item (may be wrapped in Statement::Expression)
    let expr = match &items[0] {
        crate::ast::Item::Expression(expr, _) => expr,
        crate::ast::Item::Statement(crate::ast::Statement::Expression(expr, _), _) => expr,
        other => panic!("expected expression item, got {:?}", other),
    };
    // The expression should be Annotated(@retry, Annotated(@timeout, fetch()))
    if let crate::ast::Expr::Annotated {
        annotation, target, ..
    } = expr
    {
        assert_eq!(annotation.name, "retry");
        if let crate::ast::Expr::Annotated {
            annotation: inner_ann,
            ..
        } = target.as_ref()
        {
            assert_eq!(inner_ann.name, "timeout");
        } else {
            panic!("expected nested Annotated, got {:?}", target);
        }
    } else {
        panic!("expected Annotated expression, got {:?}", expr);
    }
}

// =========================================================================
// BUG-4: Constructor patterns without payload (None, Err without args)
// =========================================================================

#[test]
fn test_match_none_constructor_pattern_no_payload() {
    let content = r#"
        let x = match opt {
            Some(v) => v,
            None => 0
        };
    "#;
    let items = parse_program_helper(content).expect("None pattern without payload should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 2);
            if let crate::ast::Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } = &match_expr.arms[0].pattern
            {
                assert_eq!(enum_name, &None);
                assert_eq!(variant, "Some");
                match fields {
                    crate::ast::PatternConstructorFields::Tuple(pats) => {
                        assert_eq!(pats.len(), 1);
                    }
                    _ => panic!("Expected Tuple payload for Some"),
                }
            } else {
                panic!("Expected constructor pattern for Some(v)");
            }
            if let crate::ast::Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } = &match_expr.arms[1].pattern
            {
                assert_eq!(enum_name, &None);
                assert_eq!(variant, "None");
                assert!(
                    matches!(fields, crate::ast::PatternConstructorFields::Unit),
                    "None should have Unit fields, got {:?}",
                    fields
                );
            } else {
                panic!(
                    "Expected constructor pattern for None, got {:?}",
                    match_expr.arms[1].pattern
                );
            }
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_match_ok_err_constructor_patterns() {
    let content = r#"
        let x = match result {
            Ok(v) => v,
            Err(e) => -1
        };
    "#;
    let items = parse_program_helper(content).expect("Ok/Err pattern should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 2);
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

// =========================================================================
// BUG-5: Chained function calls -- expr()(args)
// =========================================================================

#[test]
fn test_chained_function_calls() {
    let content = r#"
        fn id(x) { x }
        id(id)(42)
    "#;
    let items = parse_program_helper(content).expect("Chained function calls should parse");
    assert!(items.len() >= 2, "Expected at least 2 items");
}

// =========================================================================
// BUG-7: Try operator after context expression in parens
// =========================================================================

#[test]
fn test_try_operator_after_context_expr() {
    let content = r#"
        let x = (risky() !! "context")?;
    "#;
    let items = parse_program_helper(content)
        .expect("Try operator after parenthesized context expr should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        assert!(
            matches!(value, crate::ast::Expr::TryOperator(..)),
            "Expected TryOperator, got {:?}",
            value
        );
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

// =========================================================================
// BUG-13: Annotations before function definitions
// =========================================================================

#[test]
fn test_annotation_before_fn() {
    let content = r#"
        @my_ann
        fn foo() { 1 }
    "#;
    let items = parse_program_helper(content).expect("Annotation before fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "foo");
            assert!(
                !func_def.annotations.is_empty(),
                "Expected annotations on function"
            );
            assert_eq!(func_def.annotations[0].name, "my_ann");
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_annotation_with_args_before_fn() {
    let content = r#"
        @cache(60)
        fn compute() { 42 }
    "#;
    let items = parse_program_helper(content).expect("Annotation with args before fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "compute");
            assert_eq!(func_def.annotations[0].name, "cache");
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_object_pattern_shorthand_parses() {
    let input = r#"match p { {x, y} => x + y }"#;
    let result = ShapeParser::parse(Rule::expression, input);
    assert!(
        result.is_ok(),
        "Failed to parse shorthand object pattern: {:?}",
        result.err()
    );
}

#[test]
fn test_object_pattern_shorthand_full_program() {
    let input = r#"
        let p = { x: 5, y: 3 }
        match p {
            {x, y} where x > y => x - y,
            _ => 0
        }
    "#;
    let result = parse_program_helper(input);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

#[test]
fn test_debug_match_program_simple() {
    // Test: single arm, no where clause, no trailing comma
    let input = r#"match p { {x, y} => x + y }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(result.is_ok(), "Simple match failed: {:?}", result.err());
}

#[test]
fn test_debug_match_program_with_where() {
    // Test: with where clause, trailing comma, wildcard arm
    let input = r#"match p { {x, y} where x > y => x - y, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Match with where failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_match_program_multiline() {
    // Test: multiline with let
    let input =
        "let p = { x: 5, y: 3 }\nmatch p {\n    {x, y} where x > y => x - y,\n    _ => 0\n}";
    let result = ShapeParser::parse(Rule::program, input);
    assert!(result.is_ok(), "Multiline match failed: {:?}", result.err());
}

#[test]
fn test_debug_match_no_shorthand() {
    // Test: with explicit field patterns (no shorthand)
    let input =
        "let p = { x: 5, y: 3 }\nmatch p {\n    {x: x, y: y} where x > y => x - y,\n    _ => 0\n}";
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "No-shorthand match failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_match_trailing_comma_only() {
    // Test: trailing comma without where clause
    let input = r#"match p { x => x + 1, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Trailing comma only failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_match_where_no_comma() {
    // Test: where clause without trailing comma
    let input = r#"match p { x where x > 0 => x }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(result.is_ok(), "Where no comma failed: {:?}", result.err());
}

#[test]
fn test_debug_match_where_plus_comma() {
    // Test: where clause WITH trailing comma
    let input = r#"match p { x where x > 0 => x, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(result.is_ok(), "Where+comma failed: {:?}", result.err());
}

#[test]
fn test_debug_object_pattern_where_simple() {
    // Object pattern + where clause with simple condition
    let input = r#"match p { {x, y} where true => 1, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Obj+where simple failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_object_pattern_two_arms_no_where() {
    // Object pattern + two arms, no where
    let input = r#"match p { {x, y} => x + y, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Obj two arms no where failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_object_pattern_where_no_second_arm() {
    // Object pattern + where, single arm
    let input = r#"match p { {x, y} where x > y => x - y }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Obj+where single arm as PROGRAM failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_object_pattern_where_as_expression() {
    // Same input but parsed as expression
    let input = r#"match p { {x, y} where x > y => x - y }"#;
    let result = ShapeParser::parse(Rule::expression, input);
    assert!(
        result.is_ok(),
        "Obj+where single arm as EXPRESSION failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_object_pattern_where_gt_literal() {
    // Object pattern + where with comparison to literal (not identifier)
    let input = r#"match p { {x, y} where x > 0 => x, _ => 0 }"#;
    let result = ShapeParser::parse(Rule::program, input);
    assert!(
        result.is_ok(),
        "Obj+where>literal failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_where_xy_body_literal() {
    // Object pattern + where x>y, but body is just a literal
    let input = r#"match p { {x, y} where x > y => 1 }"#;
    let result = ShapeParser::parse(Rule::expression, input);
    assert!(
        result.is_ok(),
        "where x>y body=1 failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_where_xy_body_ident() {
    // Object pattern + where x>y, body is single ident
    let input = r#"match p { {x, y} where x > y => x }"#;
    let result = ShapeParser::parse(Rule::expression, input);
    assert!(
        result.is_ok(),
        "where x>y body=x failed: {:?}",
        result.err()
    );
}

#[test]
fn test_debug_ident_pattern_where_xy() {
    // identifier pattern + where x > y - same where expr, different pattern
    let input = r#"match p { z where x > y => x - y }"#;
    let result = ShapeParser::parse(Rule::expression, input);
    assert!(
        result.is_ok(),
        "ident pattern where x>y failed: {:?}",
        result.err()
    );
}

#[test]
fn test_match_qualified_option_standalone() {
    // Qualified enum variant paths must work in patterns (Option::Some, Option::None, etc.)
    let match_only = r#"match x { Option::Some(n) => 1, Option::None => 0, }"#;
    let result1 = ShapeParser::parse(Rule::match_expr, match_only);
    assert!(
        result1.is_ok(),
        "match_expr rule should parse qualified option: {:?}",
        result1.err()
    );

    let content = r#"
        let x = Option::None
        match x {
            Option::Some(n) => 1,
            Option::None => 0,
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Standalone match with qualified Option patterns should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_match_qualified_result_patterns() {
    let match_only = r#"match x { Result::Ok(v) => v, Result::Err(e) => 0, }"#;
    let result = ShapeParser::parse(Rule::match_expr, match_only);
    assert!(
        result.is_ok(),
        "match_expr should parse qualified Result patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_enum_variant_path_with_keyword_variant() {
    // Some, None, Ok, Err are keywords but must work as variant names after ::
    for input in &["Option::Some", "Option::None", "Result::Ok", "Result::Err"] {
        let result = ShapeParser::parse(Rule::enum_variant_path, input);
        assert!(
            result.is_ok(),
            "enum_variant_path should parse '{}': {:?}",
            input,
            result.err()
        );
    }
}
