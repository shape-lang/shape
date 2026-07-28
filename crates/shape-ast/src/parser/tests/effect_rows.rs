//! Parser tests for the effect-row clause (ADR-014 §8.3, grill Q4a).
//!
//! What the `!` clause actually needs to coexist with `!!` and `!=`: nothing.
//! A row begins with `{` or an identifier, so neither operator can start one,
//! and `effect_clause?` simply backtracks. That was verified by deleting the
//! negative lookahead and re-running these tests — they stayed green — so
//! this module does not claim the lookahead is what saves `!!`.
//!
//! What genuinely collides is a BARE BINDER row against a following `!expr`
//! statement, and `a_bare_binder_after_a_function_typed_cast_wins_over_a_\
//! following_unary_not` records that behaviour rather than asserting a
//! protection that does not exist.
//!
//! Tests that exercise the clause slot must cast to a FUNCTION type. A cast
//! to `string` never reaches `effect_clause` at all, so a negative test
//! written that way passes for the wrong reason.

use super::super::*;
use crate::ast::{EffectRowAnnotation, Item, TypeAnnotation, TypeParam};
use crate::error::{Result, ShapeError};

fn parse_items(input: &str) -> Result<Vec<Item>> {
    let pairs = ShapeParser::parse(Rule::program, input).map_err(|e| ShapeError::ParseError {
        message: e.to_string(),
        location: None,
    })?;
    let mut items = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::item {
                    items.push(parse_item(inner)?);
                }
            }
        }
    }
    Ok(items)
}

fn function(input: &str) -> crate::ast::FunctionDef {
    let items = parse_items(input).unwrap_or_else(|e| panic!("parse failed: {e}\nsource: {input}"));
    for item in items {
        if let Item::Function(func, _) = item {
            return func;
        }
    }
    panic!("no function item parsed from: {input}");
}

fn atom_names(row: &EffectRowAnnotation) -> Vec<String> {
    match row {
        EffectRowAnnotation::Atoms { names, .. } => names.clone(),
        EffectRowAnnotation::Param { name, .. } => {
            panic!("expected an atom set, got the binder `{name}`")
        }
    }
}

#[test]
fn declared_row_on_a_return_type_parses() {
    let func = function(r#"fn read_config(path: string) -> string ! {FsRead} { return path }"#);
    let row = func.effect_row.expect("declared row was dropped");
    assert_eq!(atom_names(&row), vec!["FsRead".to_string()]);
}

#[test]
fn multi_atom_rows_keep_every_atom() {
    let func = function(r#"fn fetch(u: string) -> string ! {NetConnect, FsWrite} { return u }"#);
    let row = func.effect_row.expect("declared row was dropped");
    assert_eq!(
        atom_names(&row),
        vec!["NetConnect".to_string(), "FsWrite".to_string()]
    );
}

#[test]
fn explicit_purity_is_an_empty_atom_set_not_an_absent_row() {
    let func = function(r#"fn hash(s: string) -> int ! {} { return 1 }"#);
    let row = func
        .effect_row
        .expect("`! {}` must produce a row, not None");
    assert!(
        atom_names(&row).is_empty(),
        "`! {{}}` is the explicit purity claim: a row with no atoms"
    );
}

#[test]
fn an_omitted_clause_leaves_no_row_at_all() {
    let func = function(r#"fn hash(s: string) -> int { return 1 }"#);
    assert!(
        func.effect_row.is_none(),
        "an omitted clause must not be confused with `! {{}}`"
    );
}

#[test]
fn a_declaration_without_a_return_type_can_still_declare_a_row() {
    let func = function(r#"fn touch(p: string) ! {FsWrite} { return }"#);
    let row = func.effect_row.expect("declared row was dropped");
    assert_eq!(atom_names(&row), vec!["FsWrite".to_string()]);
}

#[test]
fn effect_binders_parse_in_a_generic_parameter_list() {
    let func = function(r#"fn apply<T, effect F>(f: fn() -> T ! F) -> T ! F { return f() }"#);
    let params = func.type_params.expect("expected type params");
    assert_eq!(params.len(), 2);
    assert!(matches!(&params[0], TypeParam::Type { name, .. } if name == "T"));
    match &params[1] {
        TypeParam::Effect { name, .. } => assert_eq!(name, "F"),
        other => panic!("expected `effect F` binder, got {other:?}"),
    }

    // The declaration's own row references the binder.
    match func.effect_row.expect("declared row was dropped") {
        EffectRowAnnotation::Param { name, .. } => assert_eq!(name, "F"),
        other => panic!("expected the binder `F` as the declared row, got {other:?}"),
    }

    // So does the function-typed parameter.
    let param_ty = func.params[0]
        .type_annotation
        .as_ref()
        .expect("callback parameter lost its annotation");
    match param_ty {
        TypeAnnotation::Function { effects, .. } => {
            match effects.as_deref().expect("callback row was dropped") {
                EffectRowAnnotation::Param { name, .. } => assert_eq!(name, "F"),
                other => panic!("expected binder `F` on the callback type, got {other:?}"),
            }
        }
        other => panic!("expected a function type for `f`, got {other:?}"),
    }
}

#[test]
fn a_type_parameter_merely_named_like_the_keyword_is_still_a_type_parameter() {
    // `effect_keyword` is atomic with a trailing boundary check, so `effectful`
    // must not be read as `effect` + `ful`.
    let func = function(r#"fn id<effectful>(x: effectful) -> effectful { return x }"#);
    let params = func.type_params.expect("expected type params");
    match &params[0] {
        TypeParam::Type { name, .. } => assert_eq!(name, "effectful"),
        other => panic!("`effectful` must parse as an ordinary type param, got {other:?}"),
    }
}

#[test]
fn the_paren_only_function_type_spelling_still_parses() {
    let func = function(r#"fn hof(f: (int) -> bool) -> bool { return f(1) }"#);
    let param_ty = func.params[0].type_annotation.as_ref().unwrap();
    match param_ty {
        TypeAnnotation::Function { effects, .. } => assert!(effects.is_none()),
        other => panic!("expected a function type, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// `!` versus `!!` and `!=` — the collision the grammar has to survive
// ---------------------------------------------------------------------------

#[test]
fn the_error_context_operator_after_a_function_typed_cast_is_not_a_row() {
    // The cast target is a FUNCTION type, so the effect-clause slot really is
    // attempted here — a cast to `string` would never reach it, and a test
    // written that way proves nothing.
    let func = function(
        r#"fn ctx(v: string) -> string {
            let out = v as fn(int) -> bool !! "callback";
            return v
        }"#,
    );
    assert!(
        func.effect_row.is_none(),
        "`!!` must not be captured as a declared row"
    );
    assert_returned_callback_row_is_absent(&func);
}

#[test]
fn not_equals_after_a_function_typed_cast_is_not_an_effect_row() {
    let func = function(
        r#"fn cmp(f: int, b: int) -> bool {
            return (f as fn(int) -> bool) != b
        }"#,
    );
    assert!(func.effect_row.is_none());
}

#[test]
fn a_bare_binder_after_a_function_typed_cast_wins_over_a_following_unary_not() {
    // HAZARD, recorded deliberately rather than papered over.
    //
    // `! F` is a legal row and `!flag` is a legal statement, and Shape does
    // not require statement terminators. In a cast position the grammar has
    // no way to tell them apart: pest's implicit whitespace between
    // `type_annotation` and the optional clause spans newlines, and the
    // clause rule cannot look behind itself to see one. PEG resolves it in
    // favour of the row.
    //
    // This is NOT silent: `flag` is not an `effect` binder in scope, so
    // resolution rejects it by name. The user sees "unknown effect binder",
    // not a wrong program. A `;` after the cast disambiguates.
    //
    // The mitigation that DOES work grammatically is the `!("!" | "=")`
    // lookahead below, which keeps `!!` and `!=` out of the clause entirely.
    let source = r#"fn f(flag: bool, g: bool) -> bool {
            let h = g as fn(int) -> bool
            !flag
            return flag
        }"#;
    let func = function(source);
    assert!(
        func.effect_row.is_none(),
        "the row is not the declaration's"
    );

    let stolen = func
        .body
        .iter()
        .any(|stmt| format!("{stmt:?}").contains(r#"Param { name: "flag""#));
    assert!(
        stolen,
        "recording the actual behaviour: the bare binder wins. If a future \
         grammar change fixes this, delete the hazard note above with it."
    );
}

#[test]
fn a_semicolon_after_the_cast_removes_the_ambiguity() {
    let source = r#"fn f(flag: bool, g: bool) -> bool {
            let h = g as fn(int) -> bool;
            !flag;
            return flag
        }"#;
    assert_no_row_on_any_cast(source);
}

/// Assert that no function-type annotation anywhere in `source` picked up a
/// row. The declaration-level `effect_row` alone is not enough: a stolen `!`
/// lands on the nested function TYPE, not on the enclosing declaration.
fn assert_no_row_on_any_cast(source: &str) {
    let items = parse_items(source).expect("parse failed");
    let mut found_function_type = false;
    for item in &items {
        if let Item::Function(func, _) = item {
            for stmt in &func.body {
                let rendered = format!("{stmt:?}");
                if rendered.contains("Function {") {
                    found_function_type = true;
                }
                assert!(
                    !rendered.contains("EffectRowAnnotation"),
                    "a row was captured where none was written: {rendered}"
                );
            }
        }
    }
    assert!(
        found_function_type,
        "the test source must actually contain a function-type annotation, \
         otherwise the effect-clause slot is never attempted and the test \
         proves nothing"
    );
}

fn assert_returned_callback_row_is_absent(func: &crate::ast::FunctionDef) {
    for stmt in &func.body {
        let rendered = format!("{stmt:?}");
        assert!(
            !rendered.contains("EffectRowAnnotation"),
            "a row was captured where none was written: {rendered}"
        );
    }
}

#[test]
fn a_row_and_an_error_context_can_coexist_in_one_function() {
    let func = function(
        r#"fn load(p: string) -> string ! {FsRead} {
            let raw = read(p) !! "loading";
            return raw
        }"#,
    );
    let row = func.effect_row.expect("declared row was dropped");
    assert_eq!(atom_names(&row), vec!["FsRead".to_string()]);
}

#[test]
fn the_clause_binds_to_the_nearest_arrow() {
    // `fn f() -> fn() -> int ! {FsRead}`: the row attaches to the RETURNED
    // function type, not to `f`. Parenthesizing is how a caller says
    // otherwise. Locking this in because it is the one place the grammar
    // makes a choice a reader could reasonably expect to go the other way.
    let func = function(r#"fn outer() -> fn() -> int ! {FsRead} { return inner }"#);
    assert!(
        func.effect_row.is_none(),
        "the row belongs to the returned function type, not to `outer`"
    );
    match func.return_type.as_ref().expect("return type dropped") {
        TypeAnnotation::Function { effects, .. } => {
            let row = effects.as_deref().expect("inner row was dropped");
            assert_eq!(atom_names(row), vec!["FsRead".to_string()]);
        }
        other => panic!("expected a function return type, got {other:?}"),
    }
}
