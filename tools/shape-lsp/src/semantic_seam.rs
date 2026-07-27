//! The LSP's consumer of the shared semantic seam (ADR-011 §6, ADR-013 §1).
//!
//! The language server does not infer definition identity or callable contracts
//! of its own here: it calls `shape_semantic_db::callable_facts_for_source` —
//! the same function the compiler CLI calls — and formats what comes back.
//! Projection is allowed; a parallel tooling copy of the semantics is not.
//!
//! This slice consumes an ephemeral session per request rather than keeping a
//! long-lived database, so the incremental win is not yet realised in the
//! editor. The facts are the point: hover shows the resolved identity that the
//! compiler publishes for the same source, and the two agree by construction.

use std::path::Path;

use shape_semantic_db::{CallableFacts, callable_facts_for_source, unit_path_for_file};
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::util::get_word_at_position;

/// Publishes the facts for the callable `name` refers to in this buffer.
pub fn callable_facts(text: &str, file: Option<&Path>, name: &str) -> Option<CallableFacts> {
    callable_facts_for_source(&unit_path_for_file(file), text, name)
}

/// Renders published facts as a hover section.
///
/// Everything shown is read from the fact: the identity, the normalized base
/// contract, and any diagnostics the seam published with it.
pub fn hover_section(facts: &CallableFacts) -> String {
    let mut section = String::from("\n\n---\n\n**Semantic facts**\n\n");
    section.push_str(&format!(
        "- definition: `{}`\n",
        facts.identity().short_hex()
    ));
    section.push_str(&format!("- unit: `{}`\n", facts.provenance.unit_path));
    section.push_str(&format!(
        "- base contract: `{}`\n",
        facts.contract().render(facts.name())
    ));
    section.push_str(&format!(
        "- facts: `{}`\n",
        facts.content_identity().short_hex()
    ));
    for diagnostic in &facts.diagnostics {
        section.push_str(&format!(
            "- {}: {}\n",
            diagnostic.code,
            diagnostic.message()
        ));
    }
    section
}

/// Appends the semantic-facts section to a hover, when the hovered word names a
/// callable the seam publishes.
///
/// Strictly additive: a hover that exists keeps everything it had, and a word
/// with no published facts is left alone.
pub fn augment_hover(
    hover: Option<Hover>,
    text: &str,
    position: Position,
    file: Option<&Path>,
) -> Option<Hover> {
    let mut hover = hover?;
    let word = get_word_at_position(text, position)?;
    let Some(facts) = callable_facts(text, file, &word) else {
        return Some(hover);
    };

    if let HoverContents::Markup(markup) = &hover.contents {
        let mut value = markup.value.clone();
        value.push_str(&hover_section(&facts));
        hover.contents = HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        });
    }
    Some(hover)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_semantic_db::SemanticSession;

    const SOURCE: &str = "fn add(a: int, b: int) -> int {\n    a + b\n}\n\nlet total = add(1, 2)\n";

    #[test]
    fn the_lsp_reads_the_published_fact_not_its_own_inference() {
        let facts = callable_facts(SOURCE, None, "add").expect("the seam publishes `add`");

        // Byte-identical to what an independently built session publishes —
        // the LSP has no second copy of the semantics.
        let mut session = SemanticSession::new();
        session.insert_unit("<buffer>", SOURCE);
        let published = session.callable_facts_of("<buffer>", "add").unwrap();
        assert_eq!(facts.content_identity(), published.content_identity());
        assert_eq!(facts.identity(), published.identity());
    }

    #[test]
    fn hover_section_renders_the_published_contract_and_identity() {
        let facts = callable_facts(SOURCE, None, "add").unwrap();
        let section = hover_section(&facts);
        assert!(
            section.contains("fn add(a: int, b: int) -> int"),
            "{section}"
        );
        assert!(section.contains(&facts.identity().short_hex()), "{section}");
        assert!(
            section.contains(&facts.content_identity().short_hex()),
            "{section}"
        );
    }

    #[test]
    fn hovering_a_callable_augments_the_existing_hover() {
        let position = Position {
            line: 4,
            character: 13,
        };
        let hover = crate::hover::get_hover(SOURCE, position, None, None, None)
            .expect("the existing hover path answers for `add`");
        let HoverContents::Markup(markup) = &hover.contents else {
            panic!("expected markup hover");
        };
        let identity = callable_facts(SOURCE, None, "add").unwrap().identity();
        assert!(
            markup.value.contains(&identity.short_hex()),
            "hover carries the published identity: {}",
            markup.value
        );
        // The pre-existing content is still there.
        assert!(markup.value.contains("add"), "{}", markup.value);
    }

    #[test]
    fn a_word_with_no_published_facts_is_left_alone() {
        let position = Position {
            line: 4,
            character: 6,
        };
        let before =
            crate::hover::get_hover_without_semantic_facts(SOURCE, position, None, None, None);
        let after = crate::hover::get_hover(SOURCE, position, None, None, None);
        assert_eq!(
            before.map(|hover| format!("{:?}", hover.contents)),
            after.map(|hover| format!("{:?}", hover.contents))
        );
    }

    #[test]
    fn unit_path_is_derived_the_same_way_for_both_consumers() {
        assert_eq!(
            unit_path_for_file(Some(Path::new("/tmp/project/main.shape"))),
            "main"
        );
        assert_eq!(unit_path_for_file(None), "<buffer>");
    }
}
