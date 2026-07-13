//! ADR-009 D2 (slice 3): read-only `shape-expansion://` virtual documents.
//!
//! A `shape-expansion://` view RENDERS the checked generated declarations
//! the declaration-discovery fixed point reserved — sourced from the SAME
//! `generated_symbol_query()` table the compiler consumes (via
//! `generated_symbols::generated_render_inputs_*`), never a second
//! expansion pass, never an LSP re-evaluator. The rendered text is an
//! INSPECTION view: it is read-only and is NEVER fed back to the parser or
//! compiler as input (this module contains no `parse_program` /
//! `compile_in_place` call — the render-only invariant is structural, and
//! pinned by [`tests::the_module_never_reparses_its_own_render`]).
//!
//! Bidirectional source-map navigation (`source ↔ virtual`) reuses the
//! per-line mapping pattern of `foreign_lsp::PositionMap` and the virtual
//! URI-scheme precedent of `module_cache` (`__shape_lsp_virtual__`): each
//! rendered declaration line maps back to its checked-declaration source
//! anchor, and any source position inside that anchor maps forward to the
//! rendered line.

use shape_ast::ast::Program;
use tower_lsp_server::ls_types::{Position, Uri};

use crate::generated_symbols::{
    GeneratedSymbolRenderInputs, generated_render_inputs_all, generated_render_inputs_at,
};
use crate::util::offset_to_line_col;

/// The read-only virtual-document URI scheme (with authority separator) for
/// checked generated declarations. D1 reserved this vocabulary for D2 slice
/// 3; introducing it here is the relaxation the row-9 sentinel now asserts
/// POSITIVELY (`expansion_provenance.rs::row9_d2_*`).
pub const EXPANSION_URI_SCHEME: &str = "shape-expansion://";

/// One rendered declaration line paired with the real-source anchor it maps
/// back to (`[anchor_start, anchor_end)` containment for the forward map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeclLineAnchor {
    virtual_line: u32,
    anchor_start: Position,
    anchor_end: Position,
}

/// A read-only `shape-expansion://` virtual document over one or more
/// checked generated declarations, with a bidirectional source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionView {
    /// The `shape-expansion://…` URI identifying this view.
    pub uri: String,
    /// The rendered, read-only content (never reparsed as compiler input).
    pub content: String,
    /// Per virtual line: the source `Position` that line maps back to, or
    /// `None` for synthetic header lines with no source origin.
    line_origins: Vec<Option<Position>>,
    /// The rendered declaration lines and their source anchors — the
    /// forward (`source → virtual`) map.
    decl_anchors: Vec<DeclLineAnchor>,
}

impl ExpansionView {
    /// Map a position in the virtual view back to real Shape source.
    /// `None` for synthetic header lines (they have no source origin).
    /// Column offsets are preserved along the mapped declaration line so a
    /// cursor mid-token round-trips.
    pub fn virtual_to_source(&self, pos: Position) -> Option<Position> {
        let origin = (*self.line_origins.get(pos.line as usize)?)?;
        Some(Position {
            line: origin.line,
            character: origin.character + pos.character,
        })
    }

    /// The real-source anchor start of the first rendered declaration, if
    /// any — the round-trip probe point for harness assertions.
    pub fn first_decl_anchor_start(&self) -> Option<Position> {
        self.decl_anchors.first().map(|a| a.anchor_start)
    }

    /// The JSON payload an LSP client receives over
    /// `workspace/executeCommand` — the stable `shape-expansion://` URI plus
    /// the rendered read-only content. Deliberately omits the internal
    /// source-map tables (`line_origins` / `decl_anchors`): navigation is a
    /// server-side capability driven by [`Self::virtual_to_source`] /
    /// [`Self::source_to_virtual`], not something the client reconstructs.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "uri": self.uri,
            "content": self.content,
        })
    }

    /// Map a real Shape source position forward into the virtual view, when
    /// it falls inside a rendered declaration's checked-decl anchor. `None`
    /// when the source position is outside every rendered declaration.
    pub fn source_to_virtual(&self, source_pos: Position) -> Option<Position> {
        for anchor in &self.decl_anchors {
            if position_in_range(source_pos, anchor.anchor_start, anchor.anchor_end) {
                let character = if source_pos.line == anchor.anchor_start.line {
                    source_pos
                        .character
                        .saturating_sub(anchor.anchor_start.character)
                } else {
                    0
                };
                return Some(Position {
                    line: anchor.virtual_line,
                    character,
                });
            }
        }
        None
    }
}

/// Half-open-ish containment: `pos` within `[start, end]` by (line, column)
/// ordering. Used for `source → virtual` anchor resolution.
fn position_in_range(pos: Position, start: Position, end: Position) -> bool {
    let at_or_after_start = pos.line > start.line
        || (pos.line == start.line && pos.character >= start.character);
    let at_or_before_end =
        pos.line < end.line || (pos.line == end.line && pos.character <= end.character);
    at_or_after_start && at_or_before_end
}

/// Sanitize a path/name fragment for a `shape-expansion://` URI segment.
fn sanitize_segment(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Build the stable `shape-expansion://<source>/<decl>` URI for a view.
pub fn expansion_uri(source_uri: &Uri, decl_name: &str) -> String {
    format!(
        "{EXPANSION_URI_SCHEME}{}/{}",
        sanitize_segment(source_uri.as_str()),
        sanitize_segment(decl_name),
    )
}

/// Render a read-only virtual view over `inputs` (one or more checked
/// generated declarations sharing an application anchor), producing the URI,
/// the rendered content, and the bidirectional source map. `text` is the
/// real Shape source, used only to resolve anchor spans to line/column
/// coordinates — never reparsed. `inputs` must be non-empty (the callers
/// only render resolved symbols).
pub(crate) fn render_expansion_view(
    source_uri: &Uri,
    text: &str,
    inputs: &[GeneratedSymbolRenderInputs],
) -> ExpansionView {
    debug_assert!(!inputs.is_empty(), "render only resolved generated symbols");
    let uri = expansion_uri(source_uri, &inputs[0].decl_name);

    let mut lines: Vec<String> = Vec::new();
    let mut line_origins: Vec<Option<Position>> = Vec::new();
    let mut decl_anchors: Vec<DeclLineAnchor> = Vec::new();

    let push_synthetic = |lines: &mut Vec<String>,
                          line_origins: &mut Vec<Option<Position>>,
                          content: String| {
        lines.push(content);
        line_origins.push(None);
    };

    push_synthetic(
        &mut lines,
        &mut line_origins,
        "// shape-expansion:// — read-only view of checked generated declaration(s)".to_string(),
    );
    push_synthetic(
        &mut lines,
        &mut line_origins,
        "// Rendered from the declaration-discovery fixed point; never reparsed as source."
            .to_string(),
    );
    push_synthetic(&mut lines, &mut line_origins, "//".to_string());

    for input in inputs {
        let (gen_line, _) = offset_to_line_col(text, input.generator.start);
        let (app_line, _) = offset_to_line_col(text, input.application.start);
        push_synthetic(
            &mut lines,
            &mut line_origins,
            format!(
                "// {} {}  [node: {}]  generator @ line {}, applied @ line {}",
                input.kind.label(),
                input.decl_name,
                input.node_path,
                gen_line + 1,
                app_line + 1,
            ),
        );

        // The rendered declaration line maps back to the checked-decl anchor.
        let (start_line, start_col) = offset_to_line_col(text, input.checked_decl.start);
        let (end_line, end_col) = offset_to_line_col(text, input.checked_decl.end);
        let anchor_start = Position {
            line: start_line,
            character: start_col,
        };
        let anchor_end = Position {
            line: end_line,
            character: end_col,
        };
        let virtual_line = lines.len() as u32;
        lines.push(input.decl_name.clone());
        line_origins.push(Some(anchor_start));
        decl_anchors.push(DeclLineAnchor {
            virtual_line,
            anchor_start,
            anchor_end,
        });

        push_synthetic(&mut lines, &mut line_origins, String::new());
    }

    let mut content = lines.join("\n");
    content.push('\n');

    ExpansionView {
        uri,
        content,
        line_origins,
        decl_anchors,
    }
}

/// Resolve the generated symbol at a call-site cursor to its read-only
/// `shape-expansion://` virtual view — the goto-into-virtual-view route.
/// Consumes the SAME shared fixed-point query as goto/references/rename (no
/// second pass); `None` when the cursor is not on a generated-symbol call
/// site. `source_uri` names the real document the generated decl lives in.
pub(crate) fn expansion_view_at(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    source_uri: &Uri,
) -> Option<ExpansionView> {
    let inputs = generated_render_inputs_at(program, text, word, offset)?;
    Some(render_expansion_view(source_uri, text, &inputs))
}

/// One read-only virtual view per generated declaration in the document
/// (outline / workspace consumption of the views). Sourced from the shared
/// query; empty when the document generates nothing.
pub(crate) fn expansion_views_all(
    program: &Program,
    text: &str,
    source_uri: &Uri,
) -> Vec<ExpansionView> {
    generated_render_inputs_all(program, text)
        .into_iter()
        .map(|input| render_expansion_view(source_uri, text, std::slice::from_ref(&input)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    /// A generating document whose annotation emits a generated METHOD.
    const METHOD_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
"#;

    fn uri() -> Uri {
        Uri::from_file_path("/test.shape").unwrap()
    }

    #[test]
    fn goto_on_generated_call_site_renders_a_read_only_virtual_view() {
        let program = parse_program(METHOD_PROGRAM).expect("parses");
        let offset = METHOD_PROGRAM.find("p.answer()").expect("call site") + 2;
        let view = expansion_view_at(&program, METHOD_PROGRAM, "answer", offset, &uri())
            .expect("a generated method call site yields a virtual view");
        assert!(
            view.uri.starts_with(EXPANSION_URI_SCHEME),
            "the view carries a shape-expansion:// URI, got {}",
            view.uri
        );
        assert!(
            view.content.contains("Point.answer"),
            "the checked generated declaration is rendered: {}",
            view.content
        );
        assert!(
            view.content.contains("read-only"),
            "the render is marked read-only: {}",
            view.content
        );
    }

    #[test]
    fn source_and_virtual_positions_round_trip_bidirectionally() {
        let program = parse_program(METHOD_PROGRAM).expect("parses");
        let offset = METHOD_PROGRAM.find("p.answer()").expect("call site") + 2;
        let view = expansion_view_at(&program, METHOD_PROGRAM, "answer", offset, &uri())
            .expect("virtual view");
        let anchor = view.decl_anchors[0];

        // source → virtual → source round-trips at the anchor start.
        let forward = view
            .source_to_virtual(anchor.anchor_start)
            .expect("the checked-decl anchor maps into the view");
        assert_eq!(forward.line, anchor.virtual_line);
        let back = view
            .virtual_to_source(forward)
            .expect("the rendered declaration line maps back to source");
        assert_eq!(
            back, anchor.anchor_start,
            "source→virtual→source is the identity at the anchor"
        );

        // virtual → source → virtual round-trips on the rendered decl line.
        let virtual_pos = Position {
            line: anchor.virtual_line,
            character: 3,
        };
        let src = view
            .virtual_to_source(virtual_pos)
            .expect("a column on the rendered decl line maps to source");
        let round = view
            .source_to_virtual(src)
            .expect("that source position maps back into the view");
        assert_eq!(
            round, virtual_pos,
            "virtual→source→virtual is the identity on the decl line"
        );
    }

    #[test]
    fn synthetic_header_lines_have_no_source_origin() {
        let program = parse_program(METHOD_PROGRAM).expect("parses");
        let offset = METHOD_PROGRAM.find("p.answer()").expect("call site") + 2;
        let view = expansion_view_at(&program, METHOD_PROGRAM, "answer", offset, &uri())
            .expect("virtual view");
        // Line 0 is the read-only banner — synthetic, no source mapping.
        assert!(
            view.virtual_to_source(Position {
                line: 0,
                character: 0
            })
            .is_none(),
            "synthetic banner lines must not map to source"
        );
    }

    #[test]
    fn an_out_of_anchor_source_position_maps_nowhere() {
        let program = parse_program(METHOD_PROGRAM).expect("parses");
        let offset = METHOD_PROGRAM.find("p.answer()").expect("call site") + 2;
        let view = expansion_view_at(&program, METHOD_PROGRAM, "answer", offset, &uri())
            .expect("virtual view");
        // Line 0 of the source is the leading blank line — outside the
        // application anchor, so it maps to no virtual line.
        assert!(
            view.source_to_virtual(Position {
                line: 0,
                character: 0
            })
            .is_none(),
            "a source position outside every rendered decl anchor maps nowhere"
        );
    }

    #[test]
    fn non_generating_call_site_yields_no_view() {
        let source = "fn f() -> int { 1 }\nlet x = f()\n";
        let program = parse_program(source).expect("parses");
        let offset = source.find("= f()").expect("call site") + 2;
        assert!(
            expansion_view_at(&program, source, "f", offset, &uri()).is_none(),
            "an ordinary call site offers no virtual view"
        );
    }

    #[test]
    fn expansion_uri_is_stable_and_scheme_prefixed() {
        let a = expansion_uri(&uri(), "Point.answer");
        let b = expansion_uri(&uri(), "Point.answer");
        assert_eq!(a, b, "the URI is a deterministic function of its inputs");
        assert!(a.starts_with(EXPANSION_URI_SCHEME));
        assert!(
            !a.contains(' '),
            "URI segments are sanitized to path-safe characters: {a}"
        );
    }

    /// The render-only invariant is structural: this module must never feed
    /// its rendered text back to the parser or compiler. Pin it by grepping
    /// the module's own source for the compiler-input entry points.
    #[test]
    fn the_module_never_reparses_its_own_render() {
        let src = include_str!("expansion_views.rs");
        // Assemble the needles from fragments so this test's own source
        // cannot satisfy them.
        let parse_needle = ["parse_", "program("].concat();
        let compile_needle = ["compile_in_", "place("].concat();
        assert!(
            !non_test_region(src).contains(&parse_needle),
            "the virtual view must never be reparsed as compiler input"
        );
        assert!(
            !non_test_region(src).contains(&compile_needle),
            "the virtual view must never be recompiled as compiler input"
        );
    }

    /// The production (non-`#[cfg(test)]`) region of this file — the test
    /// module legitimately calls `parse_program` to build fixtures, so the
    /// render-only grep scopes to code above the test module.
    fn non_test_region(src: &str) -> &str {
        let marker = ["#[cfg(", "test)]"].concat();
        match src.find(&marker) {
            Some(idx) => &src[..idx],
            None => src,
        }
    }
}
