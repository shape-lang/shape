//! The LSP's consumer end of the structured-fix channel (ADR-017 §4).
//!
//! The compiler proves a fix once, as exact spans plus replacement text, and
//! hands it out on the diagnostic. This module does two mechanical things and
//! nothing else:
//!
//! 1. stashes those fixes on the published `Diagnostic`'s `data` field, so
//!    they come back with the diagnostic in a `codeAction` request;
//! 2. projects an [`EditPlan`]'s byte spans onto LSP `TextEdit` ranges.
//!
//! It adds no fixes of its own and reinterprets nothing. A plan whose source
//! moved since the compiler proved it is refused here exactly as
//! [`EditPlan::validate`] refuses it — the LSP does not get to decide that a
//! stale edit is close enough.

use crate::util::offset_to_position;
use shape_diagnostics::{EditPlan, FixRejection, SuggestedFix};
use tower_lsp_server::ls_types::{Diagnostic, Range, TextEdit};

/// Key under which `Diagnostic.data` carries the emitter's fixes.
const FIX_DATA_KEY: &str = "shapeStructuredFixes";

/// Stash `fixes` on `diagnostic` so a later `codeAction` request can read
/// them back. A no-op when the emitter proved nothing.
///
/// Existing `data` content is preserved: the fixes occupy one key of an
/// object rather than claiming the whole field.
pub(crate) fn attach_fixes(diagnostic: &mut Diagnostic, fixes: &[SuggestedFix]) {
    if fixes.is_empty() {
        return;
    }
    let Ok(encoded) = serde_json::to_value(fixes) else {
        return;
    };

    match diagnostic.data.take() {
        Some(serde_json::Value::Object(mut map)) => {
            map.insert(FIX_DATA_KEY.to_string(), encoded);
            diagnostic.data = Some(serde_json::Value::Object(map));
        }
        previous => {
            let mut map = serde_json::Map::new();
            if let Some(previous) = previous {
                map.insert("previous".to_string(), previous);
            }
            map.insert(FIX_DATA_KEY.to_string(), encoded);
            diagnostic.data = Some(serde_json::Value::Object(map));
        }
    }
}

/// Read back the fixes [`attach_fixes`] stored.
///
/// Returns empty for a diagnostic that carries none, and for one whose
/// payload does not decode — a malformed payload is a fix the LSP does not
/// have, never a fix it improvises.
pub(crate) fn fixes_from_diagnostic(diagnostic: &Diagnostic) -> Vec<SuggestedFix> {
    diagnostic
        .data
        .as_ref()
        .and_then(|data| data.get(FIX_DATA_KEY))
        .and_then(|value| serde_json::from_value::<Vec<SuggestedFix>>(value.clone()).ok())
        .unwrap_or_default()
}

/// Project a plan's byte spans onto `TextEdit`s against `text`.
///
/// Refuses whenever [`EditPlan::validate`] refuses, so a fix proved against
/// a revision the editor no longer holds produces no code action instead of
/// a misplaced one.
pub(crate) fn plan_to_text_edits(
    text: &str,
    plan: &EditPlan,
) -> Result<Vec<TextEdit>, FixRejection> {
    plan.validate(text)?;

    Ok(plan
        .edits
        .iter()
        .map(|edit| TextEdit {
            range: Range {
                start: offset_to_position(text, edit.start() as usize),
                end: offset_to_position(text, edit.end() as usize),
            },
            new_text: edit.new_text.clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_diagnostics::StructuredEdit;
    use tower_lsp_server::ls_types::Position;

    const TEXT: &str = "fn f() {\n  match c {\n  }\n}\n";

    fn diagnostic() -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            },
            severity: None,
            code: None,
            code_description: None,
            source: Some("shape".to_string()),
            message: "irrelevant".to_string(),
            related_information: None,
            tags: None,
            data: None,
        }
    }

    fn arm_fix() -> SuggestedFix {
        let at = TEXT.find("  }").expect("closing brace line") as u32;
        SuggestedFix::new("Add missing match arms for Color", 0.9).with_edits(
            TEXT,
            vec![StructuredEdit::insertion(at, "    Blue => 1,\n")],
        )
    }

    #[test]
    fn fixes_survive_the_diagnostic_round_trip() {
        let mut diag = diagnostic();
        attach_fixes(&mut diag, &[arm_fix()]);

        // The client echoes `data` back verbatim; model that with a JSON
        // round trip rather than trusting the in-process value.
        let wire = serde_json::to_string(&diag).expect("serialize");
        let echoed: Diagnostic = serde_json::from_str(&wire).expect("deserialize");

        let recovered = fixes_from_diagnostic(&echoed);
        assert_eq!(recovered, vec![arm_fix()]);
    }

    #[test]
    fn attaching_nothing_leaves_data_untouched() {
        let mut diag = diagnostic();
        attach_fixes(&mut diag, &[]);
        assert_eq!(diag.data, None);
    }

    #[test]
    fn existing_data_is_preserved() {
        let mut diag = diagnostic();
        diag.data = Some(serde_json::json!({ "other": 7 }));
        attach_fixes(&mut diag, &[arm_fix()]);

        let data = diag.data.expect("data");
        assert_eq!(data["other"], 7);
        assert!(data.get(FIX_DATA_KEY).is_some());
    }

    #[test]
    fn undecodable_payload_yields_no_fixes() {
        let mut diag = diagnostic();
        diag.data = Some(serde_json::json!({ FIX_DATA_KEY: "not a fix list" }));
        assert!(fixes_from_diagnostic(&diag).is_empty());
    }

    #[test]
    fn spans_project_onto_line_and_character_ranges() {
        let plan = arm_fix().edit_plan.expect("plan");
        let edits = plan_to_text_edits(TEXT, &plan).expect("projects");

        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].range.start,
            Position {
                line: 2,
                character: 0
            }
        );
        assert_eq!(edits[0].range.start, edits[0].range.end, "an insertion");
        assert_eq!(edits[0].new_text, "    Blue => 1,\n");
    }

    #[test]
    fn multibyte_text_projects_to_character_columns() {
        let text = "// héllo\nx\n";
        let at = text.find('\n').expect("newline") as u32;
        let plan = EditPlan::new(text, vec![StructuredEdit::insertion(at, "!")]);

        let edits = plan_to_text_edits(text, &plan).expect("projects");
        // Eight characters precede the newline even though `é` is two bytes.
        assert_eq!(
            edits[0].range.start,
            Position {
                line: 0,
                character: 8
            }
        );
    }

    /// Tripwire 3 at the LSP boundary: a plan proved against source the
    /// editor no longer holds produces no edits at all.
    #[test]
    fn stale_plan_produces_no_text_edits() {
        let plan = arm_fix().edit_plan.expect("plan");
        let edited = format!("// a new first line\n{TEXT}");

        let rejection = plan_to_text_edits(&edited, &plan).expect_err("must refuse");
        assert!(matches!(rejection, FixRejection::SourceChanged { .. }));
    }
}
