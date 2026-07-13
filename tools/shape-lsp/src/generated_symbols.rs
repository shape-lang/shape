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

/// The syntactic kind of a callable use-site or declaration. A generated
/// METHOD (`Point.answer`) is only reachable through method-call syntax
/// (`receiver.answer(..)`); a generated free FUNCTION only through plain or
/// qualified call syntax. Kind-matching call sites against generated
/// declarations keeps a hand-written `fn answer()` (function kind) from
/// being hijacked by a generated `Point.answer` (method kind) that shares
/// the bare name — the round-1 review finding: the pre-fix bare-name gate
/// classified the ordinary function's call sites as generated, dropping
/// the true definition from goto-definition and producing a corrupting
/// rename edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallableKind {
    Method,
    Function,
}

/// The callable kind a generated declaration answers to: a qualified
/// declaration name (`Point.answer`) is a method; a bare name is a free
/// function.
fn generated_decl_kind(decl_name: &str) -> CallableKind {
    if decl_name.contains('.') {
        CallableKind::Method
    } else {
        CallableKind::Function
    }
}

/// AST call-site collector for one callable name: method calls
/// (`receiver.name(..)`), function calls (`name(..)`), and qualified calls
/// (`ns::name(..)`). AST-node based — comments and string literals are
/// invisible here (rejection row 6). Each site carries its syntactic
/// [`CallableKind`] so generated declarations only claim kind-compatible
/// sites.
struct CallSiteCollector<'a> {
    name: &'a str,
    call_spans: Vec<(Span, CallableKind)>,
}

impl Visitor for CallSiteCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        match expr {
            Expr::MethodCall { method, span, .. } if method == self.name => {
                self.call_spans.push((*span, CallableKind::Method));
            }
            Expr::FunctionCall { name, span, .. } if name == self.name => {
                self.call_spans.push((*span, CallableKind::Function));
            }
            Expr::QualifiedFunctionCall { function, span, .. } if function == self.name => {
                self.call_spans.push((*span, CallableKind::Function));
            }
            _ => {}
        }
        true
    }
}

/// The name-token spans of every AST call site of `name` in the program,
/// each tagged with its syntactic [`CallableKind`]. The token span is
/// refined WITHIN each AST-resolved call node (the last occurrence of
/// `name` followed by `(`) — a span refinement of a resolved node, not
/// symbol discovery by text.
pub(crate) fn call_site_name_spans(
    program: &Program,
    text: &str,
    name: &str,
) -> Vec<(Span, CallableKind)> {
    let mut collector = CallSiteCollector {
        name,
        call_spans: Vec::new(),
    };
    walk_program(&mut collector, program);
    let mut spans: Vec<(Span, CallableKind)> = collector
        .call_spans
        .into_iter()
        .filter_map(|(call_span, kind)| {
            name_token_span_in_call(text, call_span, name).map(|span| (span, kind))
        })
        .collect();
    spans.sort_by_key(|(span, _)| span.start);
    spans.dedup_by(|a, b| a.0 == b.0);
    spans
}

/// Hand-written (non-generated) declarations of `name` with the given
/// callable kind: top-level `fn` / foreign-fn definitions (function kind),
/// and methods in hand-written `impl` / `extend` blocks (method kind).
/// These share call syntax with a same-named generated declaration, so
/// their existence makes bare-name call sites ambiguous — the guard the
/// three entry points consult before claiming a site exclusively.
fn ordinary_declaration_spans(program: &Program, name: &str, kind: CallableKind) -> Vec<Span> {
    let mut spans = Vec::new();
    for item in &program.items {
        match (kind, item) {
            (CallableKind::Function, Item::Function(def, _)) if def.name == name => {
                spans.push(def.name_span);
            }
            (CallableKind::Function, Item::ForeignFunction(def, _)) if def.name == name => {
                spans.push(def.name_span);
            }
            (CallableKind::Function, Item::Module(module, _)) => {
                for module_item in &module.items {
                    match module_item {
                        Item::Function(def, _) if def.name == name => spans.push(def.name_span),
                        Item::ForeignFunction(def, _) if def.name == name => {
                            spans.push(def.name_span);
                        }
                        _ => {}
                    }
                }
            }
            (CallableKind::Method, Item::Impl(block, _)) => {
                for method in &block.methods {
                    if method.name == name {
                        spans.push(method.span);
                    }
                }
            }
            (CallableKind::Method, Item::Extend(extend, _)) => {
                for method in &extend.methods {
                    if method.name == name {
                        spans.push(method.span);
                    }
                }
            }
            _ => {}
        }
    }
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

/// The syntactic kind of the AST call site under the cursor, if any. The
/// gate that keeps ordinary symbols (same name text, different position or
/// different call syntax) on the existing text/scope providers: `None` =
/// not a call-site position; the kind lets the caller claim only generated
/// declarations reachable through that call syntax.
fn call_site_kind_at(sites: &[(Span, CallableKind)], offset: usize) -> Option<CallableKind> {
    sites
        .iter()
        .find(|(span, _)| offset >= span.start && offset <= span.end)
        .map(|(_, kind)| *kind)
}

/// Go-to-definition over generated symbols (Decision 68 LSP behavior 1):
/// when the cursor sits on a KIND-COMPATIBLE call site of a generated
/// declaration, the response opens the CHECKED generated declaration
/// (anchored at its application site until D2 virtual documents) and links
/// the source application + the generator definition. `None` = not a
/// generated-symbol position (including a call site whose syntax cannot
/// reach any generated declaration — e.g. a plain `answer()` call when
/// only the METHOD `Point.answer` is generated); the caller falls through
/// to the existing providers.
///
/// When a hand-written declaration of the SAME callable kind shares the
/// bare name (a hand-written method colliding with a generated method),
/// the call site is ambiguous without receiver-type resolution: the answer
/// is the coarse-but-sound candidate SET — generated provenance PLUS the
/// hand-written declaration — so the true definition is never excluded.
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
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    let compiler = compile_for_generated_symbol_queries(program, text);
    let matches: Vec<_> = compiler
        .generated_symbol_query()
        .symbols_named(word)
        .into_iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == cursor_kind)
        .collect();
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
    for span in ordinary_declaration_spans(program, word, cursor_kind) {
        push_unique(&mut locations, location_from_span(uri, text, span));
    }
    Some(GotoDefinitionResponse::Array(locations))
}

/// Find-references over generated symbols (Decision 68 LSP behavior 3):
/// every KIND-COMPATIBLE AST call site of the generated declaration plus
/// its application site, resolved via the compiler-issued SymbolId — never
/// the text-scan fallback (rejection row 6). `None` = not a
/// generated-symbol position (call sites whose syntax cannot reach a
/// generated declaration fall through to the existing providers).
///
/// When a hand-written declaration of the SAME callable kind shares the
/// bare name, the sites are ambiguous without receiver-type resolution:
/// the generated path abstains (`None`) rather than claim references that
/// may belong to the hand-written symbol.
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
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    let compiler = compile_for_generated_symbol_queries(program, text);
    let matches: Vec<_> = compiler
        .generated_symbol_query()
        .symbols_named(word)
        .into_iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == cursor_kind)
        .collect();
    if matches.is_empty() {
        return None;
    }
    if !ordinary_declaration_spans(program, word, cursor_kind).is_empty() {
        return None;
    }
    let mut locations: Vec<Location> = Vec::new();
    for (span, kind) in &sites {
        if *kind == cursor_kind {
            push_unique(&mut locations, location_from_span(uri, text, *span));
        }
    }
    for provenance in &matches {
        push_unique(
            &mut locations,
            location_from_span(uri, text, provenance.application.span()),
        );
    }
    Some(locations)
}

/// ADR-009 D1 (S6, Decision 68): rename classification of a generated
/// symbol, derived from its `ExpansionIdentity`/`GeneratedOrigin` provenance
/// (generator + application anchors from the compiler query surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedRenameClassification {
    /// The name is an EXPLICIT SOURCE BINDER: the expansion takes it from
    /// source (the name token appears inside the generator definition or
    /// the application anchor). Rename edits ONLY these source occurrences
    /// — the expansion recomputes; generated ranges receive zero edits.
    SourceBinder {
        /// Name-token occurrences inside the compiler-provided generator /
        /// application anchors (a span refinement of compiler-resolved
        /// anchors, not symbol discovery by text scan).
        binder_spans: Vec<Span>,
        /// AST-resolved call-site name tokens of the generated symbol.
        call_site_spans: Vec<Span>,
        /// Checked-decl anchors — rename edits must never land here.
        generated_ranges: Vec<Span>,
    },
    /// The name is WHOLLY GENERATOR-CONTROLLED: it is computed by the
    /// generator (its token appears in no source anchor of the expansion).
    /// Never a text edit; the caller reports the fact and links the
    /// generator definition.
    GeneratorControlled {
        decl_names: Vec<String>,
        generator_span: Span,
    },
}

/// Every identifier-bounded occurrence of `name` INSIDE a compiler-provided
/// anchor span — the source-binder detector. This refines a span the
/// provenance query surface already resolved; it never scans the document.
fn binder_token_spans_in(text: &str, anchor: Span, name: &str) -> Vec<Span> {
    let end = anchor.end.min(text.len());
    let Some(slice) = text.get(anchor.start..end) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(found) = slice[from..].find(name) {
        let at = from + found;
        let before_is_boundary = at == 0
            || slice[..at]
                .chars()
                .next_back()
                .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        let after = &slice[at + name.len()..];
        let after_is_boundary = after
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        if before_is_boundary && after_is_boundary {
            spans.push(Span::new(anchor.start + at, anchor.start + at + name.len()));
        }
        from = at + name.len();
    }
    spans
}

/// Classify a rename request over a generated symbol (Decision 68
/// identity-controlled rename). `None` = the position does not target a
/// generated symbol — ordinary rename applies. The classification is
/// derived from the compiler-issued provenance: a name whose token appears
/// inside the expansion's generator or application anchor is an explicit
/// source binder (renaming it there recomputes the expansion); a name whose
/// token appears in NO source anchor is generator-controlled. When several
/// generated symbols answer to one name, every one must be source-bound for
/// a text rename — otherwise the coarse-but-sound answer is
/// generator-controlled (a partial text edit would desynchronize the rest).
pub fn classify_generated_rename(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
) -> Option<GeneratedRenameClassification> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    let compiler = compile_for_generated_symbol_queries(program, text);
    let matches: Vec<_> = compiler
        .generated_symbol_query()
        .symbols_named(word)
        .into_iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == cursor_kind)
        .collect();
    if matches.is_empty() {
        return None;
    }
    // A hand-written declaration of the SAME callable kind shares the bare
    // name: without receiver-type resolution the call sites are ambiguous
    // between the generated and the hand-written symbol — a text edit over
    // that set would corrupt one of them (round-1 review finding). The
    // generated classification abstains; ordinary rename applies.
    if !ordinary_declaration_spans(program, word, cursor_kind).is_empty() {
        return None;
    }
    let mut binder_spans: Vec<Span> = Vec::new();
    let mut every_match_is_source_bound = true;
    for provenance in &matches {
        let mut spans = binder_token_spans_in(text, provenance.generator.span(), word);
        spans.extend(binder_token_spans_in(
            text,
            provenance.application.span(),
            word,
        ));
        if spans.is_empty() {
            every_match_is_source_bound = false;
        }
        binder_spans.extend(spans);
    }
    let generated_ranges: Vec<Span> = matches
        .iter()
        .map(|provenance| provenance.checked_decl.span())
        .collect();
    if every_match_is_source_bound {
        binder_spans.sort_by_key(|span| span.start);
        binder_spans.dedup();
        // Only KIND-COMPATIBLE call sites belong to the generated symbol:
        // a plain/qualified `answer()` call next to a generated METHOD
        // `Point.answer` references a different (hand-written) symbol and
        // must never receive a rename edit.
        let call_site_spans: Vec<Span> = sites
            .iter()
            .filter(|(_, kind)| *kind == cursor_kind)
            .map(|(span, _)| *span)
            .collect();
        Some(GeneratedRenameClassification::SourceBinder {
            binder_spans,
            call_site_spans,
            generated_ranges,
        })
    } else {
        Some(GeneratedRenameClassification::GeneratorControlled {
            decl_names: matches
                .iter()
                .map(|provenance| provenance.decl_name.to_string())
                .collect(),
            generator_span: matches[0].generator.span(),
        })
    }
}

/// The source ranges where generated declarations anchor (checked-decl
/// anchors from the compiler query surface). Rejection row 5: rename edits
/// must NEVER land inside these ranges — the harness inspects rename edit
/// spans against this list. Empty when the document generates nothing.
pub fn generated_decl_ranges(program: &Program, text: &str) -> Vec<Span> {
    if !program_may_generate_symbols(program) {
        return Vec::new();
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    compiler
        .generated_symbol_query()
        .generated_symbols()
        .iter()
        .map(|provenance| provenance.checked_decl.span())
        .collect()
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
        for (span, kind) in &sites {
            assert_eq!(
                &GENERATING_PROGRAM[span.start..span.end],
                "answer",
                "the refined token span covers exactly the method name"
            );
            assert_eq!(
                *kind,
                CallableKind::Method,
                "p.answer() is method-call syntax"
            );
        }
    }

    /// The generated method name here is COMPUTED (`an{suffix}` inside an
    /// f-string snippet): its token never appears in the generator.
    const GENERATOR_CONTROLLED_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    let suffix = "swer"
    extend (f"extend {target.name} \{ method an{suffix}() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
"#;

    #[test]
    fn source_written_generated_name_classifies_as_source_binder() {
        let program = parse_program(GENERATING_PROGRAM).expect("parses");
        let offset = GENERATING_PROGRAM.find("p.answer()").expect("call site") + 2;
        let classification =
            classify_generated_rename(&program, GENERATING_PROGRAM, "answer", offset)
                .expect("generated symbol position classifies");
        let GeneratedRenameClassification::SourceBinder {
            binder_spans,
            call_site_spans,
            generated_ranges,
        } = classification
        else {
            panic!("`answer` is written in the generator: must classify as source binder");
        };
        assert!(
            !binder_spans.is_empty(),
            "the binder token inside the generator must be found"
        );
        for span in &binder_spans {
            assert_eq!(&GENERATING_PROGRAM[span.start..span.end], "answer");
        }
        assert_eq!(call_site_spans.len(), 2, "both call sites resolve");
        assert!(
            !generated_ranges.is_empty(),
            "the checked-decl anchors are reported for the zero-edits guard"
        );
    }

    #[test]
    fn computed_generated_name_classifies_as_generator_controlled() {
        let program = parse_program(GENERATOR_CONTROLLED_PROGRAM).expect("parses");
        let offset = GENERATOR_CONTROLLED_PROGRAM
            .find("p.answer()")
            .expect("call site")
            + 2;
        let classification =
            classify_generated_rename(&program, GENERATOR_CONTROLLED_PROGRAM, "answer", offset)
                .expect("generated symbol position classifies");
        let GeneratedRenameClassification::GeneratorControlled {
            decl_names,
            generator_span,
        } = classification
        else {
            panic!("a computed name must classify as generator-controlled");
        };
        assert_eq!(decl_names, vec!["Point.answer".to_string()]);
        assert!(
            GENERATOR_CONTROLLED_PROGRAM[generator_span.start..generator_span.end]
                .contains("comptime post"),
            "the generator span covers the handler definition"
        );
    }

    #[test]
    fn ordinary_symbols_do_not_classify_for_generated_rename() {
        // Ordinary call site in a generating document: not a generated
        // symbol — falls through to ordinary rename.
        let source = format!("{GENERATING_PROGRAM}fn helper() -> int {{ 7 }}\nlet h = helper()\n");
        let program = parse_program(&source).expect("parses");
        let offset = source.rfind("helper()").expect("call site") + 2;
        assert!(
            classify_generated_rename(&program, &source, "helper", offset).is_none(),
            "an ordinary symbol must not classify as a generated rename"
        );
        // Cursor NOT on a call site of the generated symbol: ordinary rename.
        let decl_offset = GENERATING_PROGRAM.find("type Point").expect("type decl");
        assert!(
            classify_generated_rename(&program, &source, "Point", decl_offset).is_none(),
            "the extend target's declaration position is ordinary rename territory"
        );
    }

    /// Round-1 review finding: hand-written functions COLLIDE with the
    /// generated method `Point.answer` on the bare name. Plain `answer()`
    /// and qualified `m::answer()` calls are FUNCTION-kind syntax — they
    /// can only resolve to the hand-written functions, never to the
    /// generated METHOD — so the generated gate must fall through and
    /// generated navigation must not leak the ordinary call sites.
    const COLLIDING_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

fn answer() -> int { 7 }

mod m {
  fn answer() -> int { 8 }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
let plain = answer()
let qualified = m::answer()
"#;

    /// Byte offset inside the `answer` token of the plain `answer()` call.
    fn plain_call_offset(text: &str) -> usize {
        text.rfind("= answer()").expect("plain call site") + 3
    }

    /// Byte offset inside the `answer` token of the qualified
    /// `m::answer()` call.
    fn qualified_call_offset(text: &str) -> usize {
        text.rfind("m::answer()").expect("qualified call site") + 4
    }

    #[test]
    fn ordinary_function_call_does_not_classify_as_generated_despite_name_collision() {
        let program = parse_program(COLLIDING_PROGRAM).expect("parses");
        let uri = Uri::from_file_path("/test.shape").unwrap();
        for offset in [
            plain_call_offset(COLLIDING_PROGRAM),
            qualified_call_offset(COLLIDING_PROGRAM),
        ] {
            assert!(
                generated_definition(&program, COLLIDING_PROGRAM, "answer", offset, &uri)
                    .is_none(),
                "a function-kind `answer` call (offset {offset}) resolves to \
                 a hand-written function: the generated gate must fall \
                 through so the ordinary providers serve the true definition"
            );
            assert!(
                generated_references(&program, COLLIDING_PROGRAM, "answer", offset, &uri)
                    .is_none(),
                "references on the ordinary call (offset {offset}) must fall through"
            );
            assert!(
                classify_generated_rename(&program, COLLIDING_PROGRAM, "answer", offset).is_none(),
                "rename on the ordinary call (offset {offset}) must fall \
                 through (a generated classification would edit the \
                 generator binder + method call sites and never the \
                 hand-written declarations)"
            );
        }
    }

    #[test]
    fn generated_method_references_exclude_colliding_ordinary_call_sites() {
        let program = parse_program(COLLIDING_PROGRAM).expect("parses");
        let method_offset = COLLIDING_PROGRAM.find("p.answer()").expect("method call") + 2;
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let locations =
            generated_references(&program, COLLIDING_PROGRAM, "answer", method_offset, &uri)
                .expect("the generated method call site classifies");
        let (plain_line, _) =
            offset_to_line_col(COLLIDING_PROGRAM, plain_call_offset(COLLIDING_PROGRAM));
        let (qualified_line, _) =
            offset_to_line_col(COLLIDING_PROGRAM, qualified_call_offset(COLLIDING_PROGRAM));
        assert!(
            locations
                .iter()
                .all(|location| location.range.start.line != plain_line
                    && location.range.start.line != qualified_line),
            "generated references must not include the ordinary `answer()` / \
             `m::answer()` call sites (they reference hand-written functions, \
             not `Point.answer`): {locations:?}"
        );
    }

    #[test]
    fn generated_rename_call_sites_exclude_colliding_ordinary_call_sites() {
        let program = parse_program(COLLIDING_PROGRAM).expect("parses");
        let method_offset = COLLIDING_PROGRAM.find("p.answer()").expect("method call") + 2;
        let classification =
            classify_generated_rename(&program, COLLIDING_PROGRAM, "answer", method_offset)
                .expect("the generated method call site classifies");
        let GeneratedRenameClassification::SourceBinder {
            call_site_spans, ..
        } = classification
        else {
            panic!("`answer` is written in the generator: source binder");
        };
        for ordinary_offset in [
            plain_call_offset(COLLIDING_PROGRAM),
            qualified_call_offset(COLLIDING_PROGRAM),
        ] {
            assert!(
                call_site_spans
                    .iter()
                    .all(|span| !(span.start <= ordinary_offset && ordinary_offset <= span.end)),
                "a generated-method rename must never edit an ordinary \
                 function-kind call site (offset {ordinary_offset}): \
                 {call_site_spans:?}"
            );
        }
        assert_eq!(
            call_site_spans.len(),
            1,
            "only the method call site belongs to the generated symbol"
        );
    }

    /// A hand-written `extend Other { method answer() }` shares the bare
    /// method name with the generated `Point.answer`: without receiver-type
    /// resolution the method-call sites are ambiguous. Goto-definition
    /// answers the coarse-but-sound candidate SET (generated provenance +
    /// the hand-written declaration — the true definition is never excluded
    /// from the answer set); references and rename abstain (a text edit or
    /// reference claim over an ambiguous set would corrupt/mislead one of
    /// the two symbols).
    const METHOD_COLLIDING_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

type Other { id: int }
extend Other {
  method answer() -> int { 7 }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let o = Other { id: 2 }
let a = p.answer()
let b = o.answer()
"#;

    #[test]
    fn method_name_collision_keeps_hand_written_declaration_in_definition_answer_set() {
        let program = parse_program(METHOD_COLLIDING_PROGRAM).expect("parses");
        let offset = METHOD_COLLIDING_PROGRAM
            .find("p.answer()")
            .expect("method call")
            + 2;
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let response =
            generated_definition(&program, METHOD_COLLIDING_PROGRAM, "answer", offset, &uri)
                .expect("the generated method still answers at a method call site");
        let GotoDefinitionResponse::Array(locations) = response else {
            panic!("generated definition answers a location array");
        };
        let hand_written = METHOD_COLLIDING_PROGRAM
            .find("method answer() -> int { 7 }")
            .expect("hand-written method");
        let (decl_line, _) = offset_to_line_col(METHOD_COLLIDING_PROGRAM, hand_written);
        assert!(
            locations
                .iter()
                .any(|location| location.range.start.line == decl_line),
            "the hand-written `Other.answer` declaration must stay in the \
             coarse-but-sound answer set: {locations:?}"
        );
    }

    #[test]
    fn method_name_collision_abstains_for_references_and_rename() {
        let program = parse_program(METHOD_COLLIDING_PROGRAM).expect("parses");
        let offset = METHOD_COLLIDING_PROGRAM
            .find("p.answer()")
            .expect("method call")
            + 2;
        let uri = Uri::from_file_path("/test.shape").unwrap();
        assert!(
            generated_references(&program, METHOD_COLLIDING_PROGRAM, "answer", offset, &uri)
                .is_none(),
            "an ambiguous method-call site must not claim references"
        );
        assert!(
            classify_generated_rename(&program, METHOD_COLLIDING_PROGRAM, "answer", offset)
                .is_none(),
            "an ambiguous method-call site must not classify for rename"
        );
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
