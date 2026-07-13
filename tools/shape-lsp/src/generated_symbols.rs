//! ADR-009 D1 (slice S5): LSP navigation over GENERATED declarations.
//!
//! Generated symbols are answered from the compiler's SymbolId/provenance
//! query surface (`BytecodeCompiler::generated_symbol_query()`, the ONE
//! query API of spec §4.1) — the Decision 66 closing rule: the LSP
//! consumes compiler query results, never a text scan, never a second
//! evaluator. Ordinary symbols fall through to the existing text/scope
//! providers untouched.
//!
//! Scope note (ticket D2): the declaration-discovery fixed point and
//! `shape-expansion://` virtual documents are out of scope — this module
//! navigates the EXISTING extend/materialization path only, and the
//! checked generated declaration anchors at its application site until D2
//! gives it addressable virtual text.

use shape_ast::ast::{Expr, Item, Program, Span};
use shape_runtime::visitor::{Visitor, walk_program};
use tower_lsp_server::ls_types::{
    DocumentSymbol, GotoDefinitionResponse, Location, Position, Range, SymbolInformation,
    SymbolKind, Uri,
};

use crate::util::offset_to_line_col;

/// Structural pre-filter: a document can only hold generated declarations
/// when some item carries an annotation application or a top-level
/// `comptime { }` block exists (the only producers on the existing
/// extend/materialization path). Purely AST-shape based — NOT a text scan —
/// and only a compile-avoidance gate: a `false` skips the compiler query
/// for documents that cannot generate.
pub(crate) fn program_may_generate_symbols(program: &Program) -> bool {
    program.items.iter().any(|item| match item {
        Item::Comptime(..) => true,
        Item::Function(def, _) => !def.annotations.is_empty(),
        Item::ForeignFunction(def, _) => !def.annotations.is_empty(),
        Item::StructType(def, _) => !def.annotations.is_empty(),
        Item::Enum(def, _) => !def.annotations.is_empty(),
        Item::Trait(def, _) => !def.annotations.is_empty(),
        Item::Module(def, _) => !def.annotations.is_empty(),
        _ => false,
    })
}

/// Compile the document through the SAME compiler pipeline the diagnostics
/// path uses (`analysis.rs`, RecoverAll modes) and return the compiler:
/// its `generated_symbol_query()` table then answers every navigation
/// query of this module. Compile errors are tolerated — the table holds
/// every reservation made before a failure, and navigation must keep
/// working on broken documents.
pub(crate) fn compile_for_generated_symbol_queries(
    program: &Program,
    text: &str,
) -> shape_vm::BytecodeCompiler {
    let mut compiler = shape_vm::BytecodeCompiler::new();
    compiler.set_type_diagnostic_mode(shape_vm::compiler::TypeDiagnosticMode::RecoverAll);
    compiler.set_compile_diagnostic_mode(shape_vm::compiler::CompileDiagnosticMode::RecoverAll);
    compiler.set_source(text);
    let _ = compiler.compile_in_place(program);
    compiler
}

/// AST call-site collector for one callable name: method calls
/// (`receiver.name(..)`), function calls (`name(..)`), and qualified calls
/// (`ns::name(..)`). AST-node based — comments and string literals are
/// invisible here (rejection row 6).
struct CallSiteCollector<'a> {
    name: &'a str,
    call_spans: Vec<Span>,
}

impl Visitor for CallSiteCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        match expr {
            Expr::MethodCall { method, span, .. } if method == self.name => {
                self.call_spans.push(*span);
            }
            Expr::FunctionCall { name, span, .. } if name == self.name => {
                self.call_spans.push(*span);
            }
            Expr::QualifiedFunctionCall { function, span, .. } if function == self.name => {
                self.call_spans.push(*span);
            }
            _ => {}
        }
        true
    }
}

/// The name-token spans of every AST call site of `name` in the program.
/// The token span is refined WITHIN each AST-resolved call node (the last
/// occurrence of `name` followed by `(`) — a span refinement of a resolved
/// node, not symbol discovery by text.
pub(crate) fn call_site_name_spans(program: &Program, text: &str, name: &str) -> Vec<Span> {
    let mut collector = CallSiteCollector {
        name,
        call_spans: Vec::new(),
    };
    walk_program(&mut collector, program);
    let mut spans: Vec<Span> = collector
        .call_spans
        .into_iter()
        .filter_map(|call_span| name_token_span_in_call(text, call_span, name))
        .collect();
    spans.sort_by_key(|span| span.start);
    spans.dedup();
    spans
}

/// Refine an AST call span to the callable-name token: the last occurrence
/// of `name` inside the call span that sits on identifier boundaries and is
/// immediately followed by `(` — or sits at the very end of the span
/// (method-call spans cover `receiver.method` without the argument list).
fn name_token_span_in_call(text: &str, call_span: Span, name: &str) -> Option<Span> {
    let end = call_span.end.min(text.len());
    let slice = text.get(call_span.start..end)?;
    let mut best: Option<Span> = None;
    let mut from = 0;
    while let Some(found) = slice[from..].find(name) {
        let at = from + found;
        let before_is_boundary = at == 0
            || slice[..at]
                .chars()
                .next_back()
                .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        let after = &slice[at + name.len()..];
        if before_is_boundary && (after.is_empty() || after.starts_with('(')) {
            best = Some(Span::new(
                call_span.start + at,
                call_span.start + at + name.len(),
            ));
        }
        from = at + name.len();
    }
    best
}

fn location_from_span(uri: &Uri, text: &str, span: Span) -> Location {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Location {
        uri: uri.clone(),
        range: Range {
            start: Position {
                line: start_line,
                character: start_col,
            },
            end: Position {
                line: end_line,
                character: end_col,
            },
        },
    }
}

fn push_unique(locations: &mut Vec<Location>, location: Location) {
    if !locations.contains(&location) {
        locations.push(location);
    }
}

/// Is the cursor on an AST call site of `word`? The gate that keeps
/// ordinary symbols (same name text, different position) on the existing
/// text/scope providers.
fn cursor_on_call_site(sites: &[Span], offset: usize) -> bool {
    sites
        .iter()
        .any(|span| offset >= span.start && offset <= span.end)
}

/// Go-to-definition over generated symbols (Decision 68 LSP behavior 1):
/// when the cursor sits on a call site of a generated declaration, the
/// response opens the CHECKED generated declaration (anchored at its
/// application site until D2 virtual documents) and links the source
/// application + the generator definition. `None` = not a generated-symbol
/// position; the caller falls through to the existing providers.
pub fn generated_definition(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    uri: &Uri,
) -> Option<GotoDefinitionResponse> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    if !cursor_on_call_site(&sites, offset) {
        return None;
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    let matches = compiler.generated_symbol_query().symbols_named(word);
    if matches.is_empty() {
        return None;
    }
    let mut locations = Vec::new();
    for provenance in &matches {
        push_unique(
            &mut locations,
            location_from_span(uri, text, provenance.checked_decl.span()),
        );
        push_unique(
            &mut locations,
            location_from_span(uri, text, provenance.application.span()),
        );
        push_unique(
            &mut locations,
            location_from_span(uri, text, provenance.generator.span()),
        );
    }
    Some(GotoDefinitionResponse::Array(locations))
}

/// Find-references over generated symbols (Decision 68 LSP behavior 3):
/// every AST call site of the generated declaration plus its application
/// site, resolved via the compiler-issued SymbolId — never the text-scan
/// fallback (rejection row 6). `None` = not a generated-symbol position.
pub fn generated_references(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    uri: &Uri,
) -> Option<Vec<Location>> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    if !cursor_on_call_site(&sites, offset) {
        return None;
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    let matches = compiler.generated_symbol_query().symbols_named(word);
    if matches.is_empty() {
        return None;
    }
    let mut locations: Vec<Location> = Vec::new();
    for span in &sites {
        push_unique(&mut locations, location_from_span(uri, text, *span));
    }
    for provenance in &matches {
        push_unique(
            &mut locations,
            location_from_span(uri, text, provenance.application.span()),
        );
    }
    Some(locations)
}

fn generated_symbol_kind(decl_name: &str) -> SymbolKind {
    if decl_name.contains('.') {
        SymbolKind::METHOD
    } else {
        SymbolKind::FUNCTION
    }
}

/// Workspace-symbol rows for every generated declaration matching `query`
/// (Decision 68 LSP behavior 5) — listed from the compiler table, so
/// qualified generated names that never appear as plain text still answer.
pub(crate) fn generated_workspace_symbols(
    program: &Program,
    text: &str,
    uri: &Uri,
    query: &str,
) -> Vec<SymbolInformation> {
    if !program_may_generate_symbols(program) {
        return Vec::new();
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    let query_lower = query.to_lowercase();
    compiler
        .generated_symbol_query()
        .generated_symbols()
        .iter()
        .filter(|provenance| {
            query.is_empty() || provenance.decl_name.to_lowercase().contains(&query_lower)
        })
        .map(|provenance| {
            #[allow(deprecated)]
            SymbolInformation {
                name: provenance.decl_name.to_string(),
                kind: generated_symbol_kind(provenance.decl_name),
                tags: None,
                deprecated: None,
                location: location_from_span(uri, text, provenance.checked_decl.span()),
                container_name: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    const GENERATING_PROGRAM: &str = r#"
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
let b = p.answer()
"#;

    #[test]
    fn generating_program_passes_the_structural_prefilter() {
        let program = parse_program(GENERATING_PROGRAM).expect("parses");
        assert!(program_may_generate_symbols(&program));

        let plain = parse_program("fn f() -> int { 1 }\nlet x = f()\n").expect("parses");
        assert!(
            !program_may_generate_symbols(&plain),
            "a document without annotation applications or comptime blocks cannot generate"
        );
    }

    #[test]
    fn call_site_name_spans_find_every_method_call_token() {
        let program = parse_program(GENERATING_PROGRAM).expect("parses");
        let sites = call_site_name_spans(&program, GENERATING_PROGRAM, "answer");
        assert_eq!(
            sites.len(),
            2,
            "both p.answer() call sites resolve; got {sites:?}"
        );
        for span in &sites {
            assert_eq!(
                &GENERATING_PROGRAM[span.start..span.end],
                "answer",
                "the refined token span covers exactly the method name"
            );
        }
    }

    #[test]
    fn compiled_document_answers_generated_symbol_names() {
        let program = parse_program(GENERATING_PROGRAM).expect("parses");
        let compiler = compile_for_generated_symbol_queries(&program, GENERATING_PROGRAM);
        let names: Vec<String> = compiler
            .generated_symbol_query()
            .symbols_named("answer")
            .iter()
            .map(|provenance| provenance.decl_name.to_string())
            .collect();
        assert_eq!(names, vec!["Point.answer".to_string()]);
    }
}

/// Document-symbol (outline) rows for every generated declaration,
/// anchored at the checked declaration's source anchor.
pub(crate) fn generated_document_symbols(program: &Program, text: &str) -> Vec<DocumentSymbol> {
    if !program_may_generate_symbols(program) {
        return Vec::new();
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    compiler
        .generated_symbol_query()
        .generated_symbols()
        .iter()
        .map(|provenance| {
            let (start_line, start_col) =
                offset_to_line_col(text, provenance.checked_decl.span().start);
            let (end_line, end_col) = offset_to_line_col(text, provenance.checked_decl.span().end);
            let range = Range {
                start: Position {
                    line: start_line,
                    character: start_col,
                },
                end: Position {
                    line: end_line,
                    character: end_col,
                },
            };
            #[allow(deprecated)]
            DocumentSymbol {
                name: provenance.decl_name.to_string(),
                detail: Some("generated".to_string()),
                kind: generated_symbol_kind(provenance.decl_name),
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            }
        })
        .collect()
}
