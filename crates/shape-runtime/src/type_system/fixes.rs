//! Machine-applicable fixes derived from proved type facts (ADR-017 §4).
//!
//! A fix produced here is evidence-backed: it materializes facts the checker
//! already proved (which variants a match is missing, what the scrutinee's
//! enum is called) into exact spans and replacement text. It never guesses,
//! and when the proved facts do not determine a safe edit the producer
//! returns `None` — the diagnostic still carries its advice, but no tool is
//! told it can apply anything.
//!
//! These fixes are the single source consumed by both the CLI
//! (`--diagnostics json`) and the LSP (code actions). Neither re-derives them
//! from rendered message text.

use shape_ast::ast::Span;
use shape_diagnostics::{StructuredEdit, SuggestedFix};

/// Materialize the arms a non-exhaustive match is missing.
///
/// Emits an edit only for the block form — the match's closing brace alone
/// on its own line — where the insertion point and the arm indentation are
/// both determined by the source. A single-line match (`match c { Red => 1 }`)
/// has no unambiguous place to put an arm without also repairing the
/// preceding arm's punctuation, so it gets no edit.
pub(crate) fn non_exhaustive_match_fix(
    source: &str,
    match_span: Span,
    enum_name: &str,
    missing_variants: &[String],
) -> Option<SuggestedFix> {
    if missing_variants.is_empty() || !is_path_like(enum_name) {
        return None;
    }
    if !missing_variants.iter().all(|v| is_identifier(v)) {
        return None;
    }

    let (insert_at, arm_indent) = block_arm_insert_point(source, match_span)?;

    let mut new_text = String::new();
    for variant in missing_variants {
        new_text.push_str(&format!(
            "{arm_indent}{enum_name}::{variant} => {{\n{arm_indent}}},\n"
        ));
    }

    let label = if missing_variants.len() == 1 {
        format!("Add missing match arm for {enum_name}")
    } else {
        format!("Add missing match arms for {enum_name}")
    };

    Some(
        SuggestedFix::new(label, 0.9)
            .with_edits(source, vec![StructuredEdit::insertion(insert_at, new_text)]),
    )
}

/// Byte offset at which a new arm belongs, plus the indentation an arm at
/// that position carries.
///
/// `None` unless the match's closing brace is the first non-whitespace on its
/// line, which is what makes "insert a whole line before it" well defined.
fn block_arm_insert_point(source: &str, match_span: Span) -> Option<(u32, String)> {
    let end = match_span.end.min(source.len());
    let matched = source.get(match_span.start..end)?;
    // The span ends just past the match's closing brace.
    let brace_rel = matched.rfind('}')?;
    let brace_at = match_span.start + brace_rel;

    let line_start = source[..brace_at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent = &source[line_start..brace_at];
    if !indent.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }

    Some((line_start as u32, format!("{indent}  ")))
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// An enum name may be module-qualified (`colors::Color`) but must not be a
/// type expression — a union scrutinee reports its whole type as the "enum
/// name", and `object | int::Foo` is not an arm.
fn is_path_like(name: &str) -> bool {
    !name.is_empty() && name.split("::").all(is_identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_diagnostics::EditPlan;

    const PROGRAM: &str = "enum Color { Red, Green, Blue }\n\
                           fn describe(c: Color) -> int {\n\
                           \x20 match c {\n\
                           \x20   Color::Red => 1,\n\
                           \x20 }\n\
                           }\n";

    fn match_span() -> Span {
        let start = PROGRAM.find("match c").expect("match keyword");
        let end = PROGRAM[start..].find("}\n").expect("closing brace") + start + 1;
        Span::new(start, end)
    }

    fn apply(fix: &SuggestedFix) -> String {
        fix.edit_plan
            .as_ref()
            .expect("machine-applicable")
            .apply(PROGRAM)
            .expect("applies")
    }

    #[test]
    fn materializes_one_arm_per_missing_variant() {
        let fix = non_exhaustive_match_fix(
            PROGRAM,
            match_span(),
            "Color",
            &["Green".to_string(), "Blue".to_string()],
        )
        .expect("fix");

        assert_eq!(fix.label, "Add missing match arms for Color");
        assert_eq!(
            apply(&fix),
            "enum Color { Red, Green, Blue }\n\
             fn describe(c: Color) -> int {\n\
             \x20 match c {\n\
             \x20   Color::Red => 1,\n\
             \x20   Color::Green => {\n\
             \x20   },\n\
             \x20   Color::Blue => {\n\
             \x20   },\n\
             \x20 }\n\
             }\n"
        );
    }

    #[test]
    fn arm_indent_follows_the_closing_brace() {
        let fix = non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &["Blue".to_string()])
            .expect("fix");
        let inserted = &fix.edit_plan.as_ref().unwrap().edits[0].new_text;
        assert!(
            inserted.starts_with("    Color::Blue"),
            "arm indents two past the closing brace's two: {inserted:?}"
        );
        assert_eq!(fix.label, "Add missing match arm for Color");
    }

    #[test]
    fn plan_is_bound_to_the_source_it_was_proved_against() {
        let fix = non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &["Blue".to_string()])
            .expect("fix");
        let plan: &EditPlan = fix.edit_plan.as_ref().unwrap();
        assert!(plan.validate(PROGRAM).is_ok());
        assert!(plan.validate(&format!("// edited\n{PROGRAM}")).is_err());
    }

    /// A union scrutinee reports its whole type where an enum name would go.
    /// `object | int::Missing` is not an arm, so no edit is offered.
    #[test]
    fn union_scrutinee_gets_no_edit() {
        assert!(
            non_exhaustive_match_fix(
                PROGRAM,
                match_span(),
                "object | int",
                &["Missing".to_string()],
            )
            .is_none()
        );
    }

    #[test]
    fn single_line_match_gets_no_edit() {
        let source = "fn f(c: Color) -> int { match c { Color::Red => 1 } }\n";
        let start = source.find("match c").unwrap();
        let end = source.rfind("} }").unwrap() + 1;
        assert!(
            non_exhaustive_match_fix(
                source,
                Span::new(start, end),
                "Color",
                &["Blue".to_string()]
            )
            .is_none(),
            "no unambiguous insertion point without repairing arm punctuation"
        );
    }

    #[test]
    fn no_missing_variants_means_no_fix() {
        assert!(non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &[]).is_none());
    }

    #[test]
    fn qualified_enum_name_is_accepted() {
        assert!(
            non_exhaustive_match_fix(
                PROGRAM,
                match_span(),
                "colors::Color",
                &["Blue".to_string()],
            )
            .is_some()
        );
    }
}
