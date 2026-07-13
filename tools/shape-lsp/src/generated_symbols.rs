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
    CompletionItem, CompletionItemKind, DocumentSymbol, GotoDefinitionResponse, Location, Position,
    Range, SymbolInformation, SymbolKind, Uri,
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
    /// the application anchor, outside comments). Rename edits ONLY these
    /// source occurrences — the expansion recomputes; CALL-SITE edits never
    /// land inside generated ranges. Binder spans are exempt from the
    /// generated-range guard: they live inside compiler-resolved SOURCE
    /// anchors, and in D1 the checked-decl anchor coincides with the
    /// application anchor (an application-anchored binder like
    /// `@gen("answer")` is source, not generated text).
    SourceBinder {
        /// Name-token occurrences inside the compiler-provided generator /
        /// application anchors (a span refinement of compiler-resolved
        /// anchors, not symbol discovery by text scan; comment content is
        /// excluded — comments never bind).
        binder_spans: Vec<Span>,
        /// AST-resolved call-site name tokens of the generated symbol.
        call_site_spans: Vec<Span>,
        /// Checked-decl anchors — call-site rename edits must never land
        /// here (binder spans are exempt, see the variant doc).
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

/// Comment ranges (line `//` / doc `///`, nested block `/* */`) over the
/// whole document, string-aware: a `//` or `/*` inside a string literal
/// (e.g. the extend f-string snippet) or char literal never opens a
/// comment, and a `"` inside a comment never opens string state. Comments
/// are non-semantic text — a name token inside one can never be a source
/// binder the expansion consumes (round-2 review finding 2) — so the
/// binder detector filters occurrences against this list.
fn comment_ranges(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                if bytes[i..].starts_with(b"\"\"\"") {
                    // Triple-quoted string: skip to the closing delimiter.
                    i += 3;
                    while i < bytes.len() && !bytes[i..].starts_with(b"\"\"\"") {
                        i += 1;
                    }
                    i = (i + 3).min(bytes.len());
                } else {
                    // Simple / formatted string: skip content, honoring
                    // backslash escapes (`\"` stays inside the string).
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        if bytes[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 1).min(bytes.len());
                }
            }
            b'\'' => {
                // Char literal ('x', '\n', '\u{1F600}'): skip it so a '"'
                // payload never opens string state. A quote with no nearby
                // close is not a char literal — treated as ordinary text.
                let limit = (i + 12).min(bytes.len());
                let mut j = i + 1;
                let mut closed = None;
                while j < limit {
                    match bytes[j] {
                        b'\\' => j += 2,
                        b'\'' => {
                            closed = Some(j);
                            break;
                        }
                        _ => j += 1,
                    }
                }
                i = match closed {
                    Some(close) => close + 1,
                    None => i + 1,
                };
            }
            b'/' if bytes[i + 1..].first() == Some(&b'/') => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                ranges.push(Span::new(start, i));
            }
            b'/' if bytes[i + 1..].first() == Some(&b'*') => {
                let start = i;
                let mut depth = 1usize;
                i += 2;
                while i < bytes.len() && depth > 0 {
                    if bytes[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if bytes[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                ranges.push(Span::new(start, i));
            }
            _ => i += 1,
        }
    }
    ranges
}

/// Every identifier-bounded occurrence of `name` INSIDE a compiler-provided
/// anchor span — the source-binder detector. This refines a span the
/// provenance query surface already resolved; it never scans the document.
/// Occurrences inside `comment_spans` are skipped: comments are
/// non-semantic text and can never bind a generated name (round-2 review
/// finding 2 — a decoy comment inside the generator flipped a computed
/// name to SourceBinder and produced a corrupting text rename).
fn binder_token_spans_in(
    text: &str,
    anchor: Span,
    name: &str,
    comment_spans: &[Span],
) -> Vec<Span> {
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
        let token_start = anchor.start + at;
        let token_end = token_start + name.len();
        let inside_comment = comment_spans
            .iter()
            .any(|comment| token_start < comment.end && comment.start < token_end);
        if before_is_boundary && after_is_boundary && !inside_comment {
            spans.push(Span::new(token_start, token_end));
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
    let comment_spans = comment_ranges(text);
    let mut binder_spans: Vec<Span> = Vec::new();
    let mut every_match_is_source_bound = true;
    for provenance in &matches {
        let mut spans =
            binder_token_spans_in(text, provenance.generator.span(), word, &comment_spans);
        spans.extend(binder_token_spans_in(
            text,
            provenance.application.span(),
            word,
            &comment_spans,
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

/// Completion candidates for every generated FREE FUNCTION the
/// declaration-discovery fixed point reserved (ADR-009 D2, Decision 68 LSP
/// behavior: completion sees generated decls AFTER discovery). Sourced from
/// the SAME `generated_symbol_query()` table the compiler consumes via
/// [`compile_for_generated_symbol_queries`] — no second expansion pass, no
/// LSP re-evaluator, no parallel discovery path. The
/// [`program_may_generate_symbols`] prefilter keeps non-generating documents
/// off the compile path.
///
/// Generated METHODS carry a qualified `Type.name` decl name and are only
/// reachable through receiver-typed member completion (property access), so
/// they are excluded here — a free-standing candidate list offers only the
/// bare-name free functions that a call/expression position can actually
/// resolve.
pub(crate) fn generated_symbol_completions(program: &Program, text: &str) -> Vec<CompletionItem> {
    if !program_may_generate_symbols(program) {
        return Vec::new();
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    compiler
        .generated_symbol_query()
        .generated_symbols()
        .iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == CallableKind::Function)
        .map(|provenance| CompletionItem {
            label: provenance.decl_name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("generated function".to_string()),
            ..Default::default()
        })
        .collect()
}

/// Owned render inputs for ONE generated symbol's read-only
/// `shape-expansion://` virtual view (ADR-009 D2, slice 3), extracted from
/// the SHARED `generated_symbol_query()` table — no second expansion pass,
/// no LSP re-evaluator. Spans are real-source anchors; `checked_decl` is
/// where the checked generated declaration anchors (the view renders it and
/// maps positions back to this anchor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedSymbolRenderInputs {
    pub decl_name: String,
    pub node_path: String,
    pub kind: CallableKind,
    pub checked_decl: Span,
    pub application: Span,
    pub generator: Span,
}

impl CallableKind {
    /// Rendering label for the virtual view header.
    pub(crate) fn label(self) -> &'static str {
        match self {
            CallableKind::Method => "method",
            CallableKind::Function => "function",
        }
    }
}

/// Resolve the generated symbol(s) at a call-site cursor to owned render
/// inputs for the `shape-expansion://` virtual view — the SAME shared query
/// (`generated_symbol_query()` via [`compile_for_generated_symbol_queries`])
/// that drives goto/references/rename, kind-matched exactly like
/// [`generated_definition`]. `None` = not a generated-symbol call site
/// (including a call whose syntax cannot reach any generated declaration);
/// the caller offers no virtual view and the ordinary providers serve.
pub(crate) fn generated_render_inputs_at(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
) -> Option<Vec<GeneratedSymbolRenderInputs>> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    let compiler = compile_for_generated_symbol_queries(program, text);
    let inputs: Vec<GeneratedSymbolRenderInputs> = compiler
        .generated_symbol_query()
        .symbols_named(word)
        .into_iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == cursor_kind)
        .map(|provenance| GeneratedSymbolRenderInputs {
            decl_name: provenance.decl_name.to_string(),
            node_path: provenance.node_path.render(),
            kind: generated_decl_kind(provenance.decl_name),
            checked_decl: provenance.checked_decl.span(),
            application: provenance.application.span(),
            generator: provenance.generator.span(),
        })
        .collect();
    if inputs.is_empty() {
        return None;
    }
    Some(inputs)
}

/// Owned render inputs for EVERY generated declaration in the document,
/// listed from the shared `generated_symbol_query()` table (workspace /
/// outline consumption of the virtual views). Deterministic
/// (declaration-name) order; empty when the document generates nothing.
pub(crate) fn generated_render_inputs_all(
    program: &Program,
    text: &str,
) -> Vec<GeneratedSymbolRenderInputs> {
    if !program_may_generate_symbols(program) {
        return Vec::new();
    }
    let compiler = compile_for_generated_symbol_queries(program, text);
    compiler
        .generated_symbol_query()
        .generated_symbols()
        .iter()
        .map(|provenance| GeneratedSymbolRenderInputs {
            decl_name: provenance.decl_name.to_string(),
            node_path: provenance.node_path.render(),
            kind: generated_decl_kind(provenance.decl_name),
            checked_decl: provenance.checked_decl.span(),
            application: provenance.application.span(),
            generator: provenance.generator.span(),
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

    /// A generating document whose annotation emits a generated FREE
    /// FUNCTION (`{Type}_label()`) — the flagship F1 shape.
    const FREE_FN_GENERATING_PROGRAM: &str = r#"
annotation schema_of() {
    targets: [type]
    comptime post(target, ctx) {
        extend (f"fn {target.name}_label() -> string \{ {string_lit("User schema")} \}")
    }
}

@schema_of()
type User { id: int }

User_label()
"#;

    #[test]
    fn generated_free_function_is_a_completion_candidate_from_the_shared_query() {
        let program = parse_program(FREE_FN_GENERATING_PROGRAM).expect("parses");
        let items = generated_symbol_completions(&program, FREE_FN_GENERATING_PROGRAM);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"User_label"),
            "the fixed-point-discovered generated free function must be a completion \
             candidate, sourced from generated_symbol_query(); got {labels:?}"
        );
        assert!(
            items
                .iter()
                .all(|i| i.kind == Some(CompletionItemKind::FUNCTION)),
            "generated free functions complete as functions"
        );
    }

    #[test]
    fn generated_method_is_not_a_free_standing_completion_candidate() {
        // GENERATING_PROGRAM emits the METHOD `Point.answer` (qualified name).
        // A method is reachable only through receiver-typed member completion,
        // so the free-standing candidate list must not offer it.
        let program = parse_program(GENERATING_PROGRAM).expect("parses");
        let items = generated_symbol_completions(&program, GENERATING_PROGRAM);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            !labels.contains(&"answer") && !labels.contains(&"Point.answer"),
            "a generated method is not a free-standing completion candidate; got {labels:?}"
        );
    }

    #[test]
    fn non_generating_document_yields_no_generated_completions() {
        let program = parse_program("fn f() -> int { 1 }\nlet x = f()\n").expect("parses");
        let items = generated_symbol_completions(&program, "fn f() -> int { 1 }\nlet x = f()\n");
        assert!(
            items.is_empty(),
            "a document that cannot generate pays no compile cost and offers nothing"
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
                generated_definition(&program, COLLIDING_PROGRAM, "answer", offset, &uri).is_none(),
                "a function-kind `answer` call (offset {offset}) resolves to \
                 a hand-written function: the generated gate must fall \
                 through so the ordinary providers serve the true definition"
            );
            assert!(
                generated_references(&program, COLLIDING_PROGRAM, "answer", offset, &uri).is_none(),
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

    /// Round-2 review finding 2: the generated name is COMPUTED
    /// (`an{suffix}`), but a COMMENT inside the generator mentions the
    /// computed name. Comments are non-semantic text — a token inside one
    /// can never be a source binder the expansion consumes — so the
    /// classification must stay GENERATOR-CONTROLLED (the pre-fix raw text
    /// scan flipped it to SourceBinder and emitted a corrupting text edit:
    /// comment token + call sites edited, expansion still generating the
    /// old name).
    const LINE_COMMENT_DECOY_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    // decoy: the computed method is called answer
    let suffix = "swer"
    extend (f"extend {target.name} \{ method an{suffix}() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
"#;

    /// Block-comment variant of the round-2 finding-2 decoy.
    const BLOCK_COMMENT_DECOY_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    /* decoy: the computed method is called answer */
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
    fn comment_ranges_are_string_aware_and_handle_nesting() {
        // `//` inside a string (the extend f-string snippet shape) never
        // opens a comment.
        let text = r#"let url = "http://host/answer" // real comment"#;
        let ranges = comment_ranges(text);
        assert_eq!(
            ranges.len(),
            1,
            "only the trailing line comment: {ranges:?}"
        );
        assert_eq!(&text[ranges[0].start..ranges[0].end], "// real comment");

        // Nested block comments close at the OUTER `*/`.
        let text = "let a = 1 /* outer /* inner */ still */ let b = 2";
        let ranges = comment_ranges(text);
        assert_eq!(ranges.len(), 1);
        assert_eq!(
            &text[ranges[0].start..ranges[0].end],
            "/* outer /* inner */ still */"
        );

        // Doc comments are comments; a `"` inside a comment does not open
        // string state; a char-literal `'"'` does not open string state.
        let text = "/// doc \" answer\nlet q = '\"'\n// tail answer";
        let ranges = comment_ranges(text);
        assert_eq!(ranges.len(), 2, "doc + tail line comment: {ranges:?}");
        assert!(text[ranges[0].start..ranges[0].end].starts_with("/// doc"));
        assert!(text[ranges[1].start..ranges[1].end].starts_with("// tail"));

        // Escaped quote stays inside the string: the comment-looking text
        // after it is still string content.
        let text = r#"let s = "a \" // not a comment" // yes comment"#;
        let ranges = comment_ranges(text);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(&text[ranges[0].start..ranges[0].end], "// yes comment");
    }

    #[test]
    fn comment_decoys_inside_the_generator_stay_generator_controlled() {
        for (label, source) in [
            ("line comment", LINE_COMMENT_DECOY_PROGRAM),
            ("block comment", BLOCK_COMMENT_DECOY_PROGRAM),
        ] {
            let program = parse_program(source).expect("parses");
            let offset = source.find("p.answer()").expect("call site") + 2;
            let classification = classify_generated_rename(&program, source, "answer", offset)
                .expect("generated symbol position classifies");
            assert!(
                matches!(
                    classification,
                    GeneratedRenameClassification::GeneratorControlled { .. }
                ),
                "a computed name mentioned only in a {label} inside the \
                 generator must stay generator-controlled (comments are \
                 never source binders), got {classification:?}"
            );
        }
    }

    /// Adverse-direction control for the comment filter: a name written
    /// LITERALLY inside the extend f-string snippet in the generator IS an
    /// explicit source binder (editing it recomputes the expansion) — the
    /// comment filter must not swallow string content.
    const FSTRING_BINDER_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
"#;

    #[test]
    fn fstring_snippet_binder_in_the_generator_stays_a_source_binder() {
        let program = parse_program(FSTRING_BINDER_PROGRAM).expect("parses");
        let offset = FSTRING_BINDER_PROGRAM
            .find("p.answer()")
            .expect("call site")
            + 2;
        let classification =
            classify_generated_rename(&program, FSTRING_BINDER_PROGRAM, "answer", offset)
                .expect("generated symbol position classifies");
        let GeneratedRenameClassification::SourceBinder { binder_spans, .. } = classification
        else {
            panic!(
                "a name written literally in the extend snippet is a source \
                 binder, got {classification:?}"
            );
        };
        assert!(
            binder_spans
                .iter()
                .any(|span| &FSTRING_BINDER_PROGRAM[span.start..span.end] == "answer"),
            "the snippet binder token must be reported: {binder_spans:?}"
        );
    }

    /// Round-2 review finding 1 (classification half): a name bound ONLY at
    /// the APPLICATION site (`@gen("answer")` — the annotation argument the
    /// handler splices into the extend snippet) is an explicit source
    /// binder; the binder span sits inside the application anchor.
    const APPLICATION_BINDER_PROGRAM: &str = r#"
annotation gen(mname) {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method {mname}() -> int \{ 1 \} \}")
  }
}

@gen("answer")
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
"#;

    #[test]
    fn application_argument_binder_classifies_as_source_binder_with_application_span() {
        let program = parse_program(APPLICATION_BINDER_PROGRAM).expect("parses");
        let offset = APPLICATION_BINDER_PROGRAM
            .find("p.answer()")
            .expect("call site")
            + 2;
        let classification =
            classify_generated_rename(&program, APPLICATION_BINDER_PROGRAM, "answer", offset)
                .expect("generated symbol position classifies");
        let GeneratedRenameClassification::SourceBinder { binder_spans, .. } = classification
        else {
            panic!(
                "the annotation argument `\"answer\"` is a source binder \
                 (renaming it recomputes), got {classification:?}"
            );
        };
        let application_token = APPLICATION_BINDER_PROGRAM
            .find("@gen(\"answer\")")
            .expect("application site")
            + "@gen(\"".len();
        assert!(
            binder_spans
                .iter()
                .any(|span| span.start == application_token),
            "the application-argument binder token must be reported: \
             {binder_spans:?} (expected a span starting at {application_token})"
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
