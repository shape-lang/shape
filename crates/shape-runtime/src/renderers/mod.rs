//! Content renderers for various output targets.
//!
//! Each renderer implements `ContentRenderer` to produce formatted output
//! from a `ContentNode` tree.

pub mod html;
pub mod json;
pub mod markdown;
pub mod plain;
pub mod terminal;

#[cfg(test)]
mod cross_renderer_tests {
    use crate::content_renderer::ContentRenderer;
    use crate::renderers::{
        html::HtmlRenderer, json::JsonRenderer, markdown::MarkdownRenderer, plain::PlainRenderer,
        terminal::TerminalRenderer,
    };
    use shape_value::content::*;

    /// Build a representative ContentNode tree for cross-renderer testing.
    fn sample_tree() -> ContentNode {
        ContentNode::Fragment(vec![
            ContentNode::plain("Hello ")
                .with_bold()
                .with_fg(Color::Named(NamedColor::Red)),
            ContentNode::plain("world").with_italic(),
            ContentNode::Table(ContentTable {
                headers: vec!["Name".into(), "Value".into()],
                rows: vec![
                    vec![ContentNode::plain("alpha"), ContentNode::plain("1")],
                    vec![ContentNode::plain("beta"), ContentNode::plain("2")],
                ],
                border: BorderStyle::Rounded,
                max_rows: None,
                column_types: Some(vec!["string".into(), "number".into()]),
                total_rows: None,
                sortable: true,
            }),
            ContentNode::Code {
                language: Some("rust".into()),
                source: "fn main() {}".into(),
            },
        ])
    }

    #[test]
    fn terminal_contains_ansi_codes() {
        let tree = sample_tree();
        let output = TerminalRenderer::new().render(&tree);
        assert!(
            output.contains("\x1b["),
            "terminal output should contain ANSI codes"
        );
        assert!(output.contains("Hello"));
        assert!(output.contains("world"));
        assert!(output.contains("alpha"));
    }

    #[test]
    fn html_contains_tags() {
        let tree = sample_tree();
        let output = HtmlRenderer::new().render(&tree);
        assert!(output.contains("<span"), "HTML should contain <span> tags");
        assert!(output.contains("<table>"), "HTML should contain <table>");
        assert!(
            output.contains("<pre><code"),
            "HTML should contain <pre><code>"
        );
        assert!(output.contains("Hello"));
    }

    #[test]
    fn plain_has_no_escape_codes() {
        let tree = sample_tree();
        let output = PlainRenderer.render(&tree);
        assert!(
            !output.contains("\x1b["),
            "plain output should not contain ANSI codes"
        );
        assert!(
            !output.contains("<span"),
            "plain output should not contain HTML tags"
        );
        assert!(output.contains("Hello"));
        assert!(output.contains("alpha"));
    }

    #[test]
    fn markdown_uses_gfm_tables() {
        let tree = sample_tree();
        let output = MarkdownRenderer.render(&tree);
        assert!(
            output.contains("| Name"),
            "markdown should use pipe table syntax"
        );
        assert!(
            output.contains("```rust"),
            "markdown should use fenced code blocks"
        );
        assert!(output.contains("Hello"));
    }

    #[test]
    fn json_is_valid_json() {
        let tree = sample_tree();
        let output = JsonRenderer.render(&tree);
        let parsed: serde_json::Value =
            serde_json::from_str(&output).expect("JSON renderer output should be valid JSON");
        assert!(parsed.get("type").is_some(), "JSON should have 'type' key");
    }

    #[test]
    fn all_renderers_agree_on_text_content() {
        let node = ContentNode::plain("consistent");
        let terminal = TerminalRenderer::new().render(&node);
        let html = HtmlRenderer::new().render(&node);
        let plain = PlainRenderer.render(&node);
        let markdown = MarkdownRenderer.render(&node);

        // All should contain the plain text (terminal/html may have extra formatting)
        assert!(terminal.contains("consistent"));
        assert!(html.contains("consistent"));
        assert!(plain.contains("consistent"));
        assert!(markdown.contains("consistent"));
    }
}
