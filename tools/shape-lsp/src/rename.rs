//! Rename support for Shape
//!
//! Provides symbol renaming across the document using text-based searching.

use crate::document::DocumentManager;
use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, offset_to_line_col, position_to_offset};
use shape_ast::ast::{ImportItems, Item, Program, Statement};
use shape_ast::parser::parse_program;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp_server::ls_types::{
    Position, PrepareRenameResponse, Range, TextEdit, Uri, WorkspaceEdit,
};

mod generated;
#[cfg(test)]
pub(crate) use generated::generated_rename_from_compiler;
pub use generated::{
    GENERATOR_CONTROLLED_NAME_RENAME_REPORT, GeneratedRename, GeneratorControlledRename,
    generated_rename,
};
pub(crate) use generated::{GeneratedRenameRequest, generated_rename_request};

/// Prepare for rename - check if the symbol at the position can be renamed
pub fn prepare_rename(text: &str, position: Position) -> Option<PrepareRenameResponse> {
    // Get the word at position
    let word = get_word_at_position(text, position)?;
    let range = get_word_range(text, position)?;

    // Check if it's a renameable symbol
    if is_keyword(&word) {
        return None;
    }

    // Check if it's a built-in function
    if is_builtin_function(&word) {
        return None;
    }

    // ADR-009 D1 (S6, rejection row 4): a wholly generator-controlled
    // generated name is never renameable as a text edit — decline at
    // prepare time so the editor does not offer the rename at all.
    if let (Some(offset), Ok(program)) = (position_to_offset(text, position), parse_program(text)) {
        if let Some(
            crate::generated_symbols::GeneratedRenameClassification::GeneratorControlled { .. },
        ) = crate::generated_symbols::classify_generated_rename(&program, text, &word, offset)
        {
            return None;
        }
    }

    Some(PrepareRenameResponse::Range(range))
}

/// Perform the rename operation.
///
/// When `cached_program` is provided, it is used as fallback when the
/// current source fails to parse.
pub fn rename(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
) -> Option<WorkspaceEdit> {
    // Validate new name
    if !is_valid_identifier(new_name) {
        return None;
    }

    // ADR-009 D1 (S6): generated symbols are classified from the compiler
    // query surface FIRST. A source-binder name renames by recomputation
    // (source occurrences only); a generator-controlled name is never a
    // text edit (the server surfaces the report; at this Option level the
    // answer is "no edit"). Ordinary symbols fall through untouched.
    match generated_rename(text, uri, position, new_name, cached_program) {
        Some(GeneratedRename::Edits(edit)) => return Some(edit),
        Some(GeneratedRename::GeneratorControlled(_)) => return None,
        None => {}
    }

    rename_after_generated_query(text, uri, position, new_name, cached_program)
}

/// Ordinary scope-aware rename after the request has conclusively classified
/// both generated captures and generated symbols from its shared compiler.
pub(crate) fn rename_after_generated_query(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
) -> Option<WorkspaceEdit> {
    if !is_valid_identifier(new_name) {
        return None;
    }
    // Get the current name
    let old_name = get_word_at_position(text, position)?;

    // Check if it's renameable
    if is_keyword(&old_name) || is_builtin_function(&old_name) {
        return None;
    }

    // Parse to extract symbols and verify this is a valid symbol
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => match cached_program {
            Some(p) => p.clone(),
            None => return None,
        },
    };

    // Convert cursor position to byte offset for scope-aware lookup
    let offset = position_to_offset(text, position)?;
    let tree = crate::scope::ScopeTree::build(&program, text);

    // Use scope-aware resolution to find all references to this binding
    let edits = if let Some(spans) = tree.references_of(offset) {
        spans
            .into_iter()
            .map(|(start, end)| {
                let (start_line, start_col) = offset_to_line_col(text, start);
                let (end_line, end_col) = offset_to_line_col(text, end);
                TextEdit {
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
                    new_text: new_name.to_string(),
                }
            })
            .collect()
    } else {
        // Fallback to text-based search
        find_symbol_occurrences(text, &old_name, new_name)
    };

    if edits.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// W2.6 — Cross-file rename.
///
/// Performs the same scope-aware in-file rename as [`rename`], then for
/// module-scope symbols extends edits to other open documents + workspace
/// `.shape` files. Mirrors the cross-file find-references algorithm in
/// `definition.rs::get_references_cross_file`: only top-level
/// (module-scope-visible) symbols cascade; locally-shadowing inner bindings
/// are excluded by `ScopeTree` semantics in each file.
///
/// Returns `None` if the symbol is not renameable or no edits were
/// produced.
#[allow(clippy::too_many_arguments)]
pub fn rename_cross_file(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
    documents: Option<&DocumentManager>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&Path>,
) -> Option<WorkspaceEdit> {
    rename_cross_file_impl(
        text,
        uri,
        position,
        new_name,
        cached_program,
        documents,
        module_cache,
        workspace_root,
        false,
    )
}

/// Cross-file rename after the request's generated query has already returned
/// `NotCapture` and generated-symbol classification has also abstained.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rename_cross_file_after_generated_query(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
    documents: Option<&DocumentManager>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&Path>,
) -> Option<WorkspaceEdit> {
    rename_cross_file_impl(
        text,
        uri,
        position,
        new_name,
        cached_program,
        documents,
        module_cache,
        workspace_root,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn rename_cross_file_impl(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
    documents: Option<&DocumentManager>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&Path>,
    generated_query_complete: bool,
) -> Option<WorkspaceEdit> {
    // Same-file edits via the existing scope-aware path.
    let mut workspace_edit = if generated_query_complete {
        rename_after_generated_query(text, uri, position, new_name, cached_program)?
    } else {
        rename(text, uri, position, new_name, cached_program)?
    };

    // Determine the symbol name for cross-file scan.
    let Some(old_name) = get_word_at_position(text, position) else {
        return Some(workspace_edit);
    };

    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => match cached_program {
            Some(p) => p.clone(),
            None => return Some(workspace_edit),
        },
    };

    if !is_module_scope_symbol_in_rename(&program, &old_name) {
        // Local-scope binding — same-file rename is sufficient.
        return Some(workspace_edit);
    }

    let changes_map = workspace_edit.changes.get_or_insert_with(HashMap::new);

    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    if let Some(current_path) = uri.to_file_path() {
        visited.insert(current_path.into_owned());
    }

    if let Some(docs) = documents {
        for other_uri in docs.all_uris() {
            if &other_uri == uri {
                continue;
            }
            let Some(other_path_cow) = other_uri.to_file_path() else {
                continue;
            };
            let other_path = other_path_cow.into_owned();
            if !visited.insert(other_path.clone()) {
                continue;
            }
            let Some(other_doc) = docs.get(&other_uri) else {
                continue;
            };
            let other_text = other_doc.text();
            let edits = collect_module_scope_edits_in_file(&other_text, &old_name, new_name);
            if !edits.is_empty() {
                changes_map
                    .entry(other_uri)
                    .or_insert_with(Vec::new)
                    .extend(edits);
            }
        }
    }

    if let (Some(cache), Some(root)) = (module_cache, workspace_root) {
        let _ = cache;
        for path in cache.enumerate_workspace_shape_files(root) {
            if !visited.insert(path.clone()) {
                continue;
            }
            let Some(other_uri) = Uri::from_file_path(&path) else {
                continue;
            };
            let Ok(other_text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let edits = collect_module_scope_edits_in_file(&other_text, &old_name, new_name);
            if !edits.is_empty() {
                changes_map
                    .entry(other_uri)
                    .or_insert_with(Vec::new)
                    .extend(edits);
            }
        }
    }

    Some(workspace_edit)
}

/// Mirrors `definition::is_module_scope_symbol` — kept local to avoid a
/// pub-cross-module dependency between sibling LSP modules.
fn is_module_scope_symbol_in_rename(program: &Program, name: &str) -> bool {
    for item in &program.items {
        match item {
            Item::Function(func, _) if func.name == name => return true,
            Item::ForeignFunction(func, _) if func.name == name => return true,
            Item::Trait(t, _) if t.name == name => return true,
            Item::Enum(e, _) if e.name == name => return true,
            Item::TypeAlias(ta, _) if ta.name == name => return true,
            Item::StructType(s, _) if s.name == name => return true,
            Item::VariableDecl(decl, _) => {
                for (n, _) in crate::symbols::get_pattern_names(&decl.pattern) {
                    if n == name {
                        return true;
                    }
                }
            }
            Item::Statement(Statement::VariableDecl(decl, _), _) => {
                for (n, _) in crate::symbols::get_pattern_names(&decl.pattern) {
                    if n == name {
                        return true;
                    }
                }
            }
            Item::Import(import_stmt, _) => match &import_stmt.items {
                ImportItems::Named(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        if local == name {
                            return true;
                        }
                    }
                }
                ImportItems::Namespace {
                    name: ns_name,
                    alias,
                } => {
                    let local = alias.as_ref().unwrap_or(ns_name);
                    if local == name {
                        return true;
                    }
                }
            },
            _ => {}
        }
    }
    false
}

/// Collect TextEdits for module-scope occurrences of `old_name` in
/// `text`, replacing each with `new_name`. Uses ScopeTree to skip
/// locally-shadowing inner bindings.
fn collect_module_scope_edits_in_file(text: &str, old_name: &str, new_name: &str) -> Vec<TextEdit> {
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return Vec::new();
            }
            partial.into_program()
        }
    };

    if !is_module_scope_symbol_in_rename(&program, old_name) {
        return Vec::new();
    }

    let tree = crate::scope::ScopeTree::build(&program, text);
    let Some(root) = tree.scopes.first() else {
        return Vec::new();
    };

    let mut edits = Vec::new();
    for binding in &root.bindings {
        if binding.name != old_name {
            continue;
        }
        let mut push = |span: (usize, usize)| {
            let (sl, sc) = offset_to_line_col(text, span.0);
            let (el, ec) = offset_to_line_col(text, span.1);
            edits.push(TextEdit {
                range: Range {
                    start: Position {
                        line: sl,
                        character: sc,
                    },
                    end: Position {
                        line: el,
                        character: ec,
                    },
                },
                new_text: new_name.to_string(),
            });
        };
        push(binding.def_span);
        for span in &binding.references {
            push(*span);
        }
    }
    edits
}

/// Find all occurrences of a symbol and create edits to rename them
fn find_symbol_occurrences(text: &str, old_name: &str, new_name: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();

    // Find all word-boundary matches
    let name_len = old_name.len();

    for (i, _) in text.match_indices(old_name) {
        // Check word boundaries
        let before_ok = i == 0 || !is_identifier_char(text.chars().nth(i - 1).unwrap_or(' '));
        let after_ok = i + name_len >= text.len()
            || !is_identifier_char(text.chars().nth(i + name_len).unwrap_or(' '));

        if before_ok && after_ok {
            let (start_line, start_col) = offset_to_line_col(text, i);
            let (end_line, end_col) = offset_to_line_col(text, i + name_len);

            edits.push(TextEdit {
                range: Range {
                    start: Position {
                        line: start_line as u32,
                        character: start_col as u32,
                    },
                    end: Position {
                        line: end_line as u32,
                        character: end_col as u32,
                    },
                },
                new_text: new_name.to_string(),
            });
        }
    }

    edits
}

/// Get the range of the word at a position
fn get_word_range(text: &str, position: Position) -> Option<Range> {
    let offset = position_to_offset(text, position)?;
    let bytes = text.as_bytes();

    // Find word start
    let mut start = offset;
    while start > 0 && is_identifier_char(bytes[start - 1] as char) {
        start -= 1;
    }

    // Find word end
    let mut end = offset;
    while end < bytes.len() && is_identifier_char(bytes[end] as char) {
        end += 1;
    }

    if start == end {
        return None;
    }

    let (start_line, start_col) = offset_to_line_col(text, start);
    let (end_line, end_col) = offset_to_line_col(text, end);

    Some(Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    })
}

/// Check if a character is valid in an identifier
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Check if a string is a valid identifier
pub(crate) fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap();

    // First character must be letter or underscore
    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    // Rest must be alphanumeric or underscore
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Check if a string is a Shape keyword
fn is_keyword(name: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "let", "var", "const", "function", "pattern", "if", "else", "while", "for", "return",
        "break", "continue", "in", "and", "or", "not", "true", "false", "None", "Some", "pub",
        "from", "type", "trait", "enum", "extend", "find", "scan", "analyze", "backtest", "alert",
        "on", "test", "stream",
    ];
    KEYWORDS.contains(&name)
}

/// Check if a name is a language-level built-in function
///
/// These are functions provided by the VM runtime, not from stdlib.
/// Stdlib functions are discovered dynamically via annotation/import discovery.
fn is_builtin_function(name: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "abs", "sqrt", "pow", "log", "exp", "sin", "cos", "tan", "min", "max", "avg", "sum", "std",
        "variance", "len", "first", "last", "at", "slice", "map", "filter", "reduce", "sort",
        "reverse", "unique", "flatten", "zip", "range", "print",
    ];
    BUILTINS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("baz123"));
        assert!(is_valid_identifier("_"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123abc"));
        assert!(!is_valid_identifier("foo-bar"));
    }

    #[test]
    fn test_is_keyword() {
        assert!(is_keyword("let"));
        assert!(is_keyword("function"));
        assert!(is_keyword("if"));

        assert!(!is_keyword("foo"));
        assert!(!is_keyword("myVar"));
    }

    #[test]
    fn test_is_builtin_function() {
        // stdlib functions (e.g. sma/ema) are not language-level builtins.
        assert!(is_builtin_function("print"));
        assert!(is_builtin_function("abs"));

        assert!(!is_builtin_function("myFunc"));
        assert!(!is_builtin_function("sma")); // Now in stdlib/finance, not builtin
    }

    #[test]
    fn test_get_word_at_position() {
        let text = "let foo = bar + baz;";

        let word = get_word_at_position(
            text,
            Position {
                line: 0,
                character: 5,
            },
        );
        assert_eq!(word, Some("foo".to_string()));

        let word = get_word_at_position(
            text,
            Position {
                line: 0,
                character: 11,
            },
        );
        assert_eq!(word, Some("bar".to_string()));
    }

    #[test]
    fn test_rename_cross_file_module_scope_fn() {
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text = "fn shared() { return 1 }\nlet a = shared()".to_string();
        let other_text = "fn shared() { return 2 }\nlet b = shared() + shared()".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        let pos = Position {
            line: 0,
            character: 3,
        };
        let edit = rename_cross_file(
            &main_text,
            &main_uri,
            pos,
            "renamed",
            None,
            Some(&docs),
            None,
            None,
        )
        .expect("rename should produce a WorkspaceEdit");

        let changes = edit.changes.expect("changes map");
        assert!(
            changes.contains_key(&main_uri),
            "main uri must be in changes"
        );
        assert!(
            changes.contains_key(&other_uri),
            "other uri must be in changes for cross-file rename"
        );
        let other_edits = &changes[&other_uri];
        // other.shape: def + 2 refs = 3 edits
        assert!(
            other_edits.len() >= 3,
            "expected ≥3 edits in /other.shape (def + 2 refs), got {}",
            other_edits.len()
        );
        for te in other_edits {
            assert_eq!(te.new_text, "renamed");
        }
    }

    #[test]
    fn test_rename_cross_file_local_binding_no_crossover() {
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text = "fn outer() {\n  let local = 1\n  return local + local\n}".to_string();
        let other_text = "fn other() {\n  let local = 5\n  return local\n}".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        let offset = main_text.find("local").unwrap();
        let (line, col) = offset_to_line_col(&main_text, offset);
        let edit = rename_cross_file(
            &main_text,
            &main_uri,
            Position {
                line,
                character: col,
            },
            "new_local",
            None,
            Some(&docs),
            None,
            None,
        )
        .expect("rename should produce edits for local scope");

        let changes = edit.changes.expect("changes");
        assert!(changes.contains_key(&main_uri));
        assert!(
            !changes.contains_key(&other_uri),
            "local-scope `local` rename must NOT touch /other.shape"
        );
    }

    // -- ADR-009 D1 (S6): rename semantics over generated symbols ---------

    /// `method answer()` is written in the generator: explicit source binder.
    const SOURCE_BINDER_PROGRAM: &str = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

// decoy: the word answer appears in this comment
let decoy = "answer"

@gen()
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
let b = p.answer()
"#;

    /// The method name is COMPUTED (`an{suffix}`): generator-controlled.
    const GENERATOR_CONTROLLED_PROGRAM: &str = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    let suffix = "swer"
    extend (extend_method_literal(target.name, f"an{suffix}", "int", 42))
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
"#;

    fn call_site_position(text: &str) -> Position {
        let offset = text.find("p.answer()").expect("call site") + 2;
        let (line, character) = offset_to_line_col(text, offset);
        Position { line, character }
    }

    /// Rejection row 5: rename on a source-binder generated name edits the
    /// generator binder token + the call sites; decoy comment/string
    /// occurrences are NOT edited (never a text scan).
    #[test]
    fn generated_source_binder_rename_edits_binder_and_call_sites_only() {
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let position = call_site_position(SOURCE_BINDER_PROGRAM);
        let edit = rename(SOURCE_BINDER_PROGRAM, &uri, position, "solution", None)
            .expect("source-binder rename produces edits");
        let edits = &edit.changes.expect("changes")[&uri];
        let lines: Vec<u32> = edits.iter().map(|e| e.range.start.line).collect();
        assert!(
            lines.contains(&4),
            "generator binder token edited: {lines:?}"
        );
        assert!(lines.contains(&16), "first call site edited: {lines:?}");
        assert!(lines.contains(&17), "second call site edited: {lines:?}");
        assert!(
            !lines.contains(&9),
            "decoy comment must not be edited: {lines:?}"
        );
        assert!(
            !lines.contains(&10),
            "decoy string must not be edited: {lines:?}"
        );
    }

    /// Rejection row 4: a generator-controlled name is never a text edit —
    /// `generated_rename` answers the named report linking the generator
    /// definition, and `rename` declines.
    #[test]
    fn generator_controlled_rename_is_the_named_report_and_never_an_edit() {
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let position = call_site_position(GENERATOR_CONTROLLED_PROGRAM);
        let outcome = generated_rename(
            GENERATOR_CONTROLLED_PROGRAM,
            &uri,
            position,
            "solution",
            None,
        )
        .expect("generated-symbol position classifies");
        let GeneratedRename::GeneratorControlled(report) = outcome else {
            panic!("computed name must be generator-controlled, got {outcome:?}");
        };
        assert!(
            report
                .message
                .contains(GENERATOR_CONTROLLED_NAME_RENAME_REPORT),
            "report carries the named const: {}",
            report.message
        );
        assert!(
            report.message.contains("Point.answer"),
            "report names the generated declaration: {}",
            report.message
        );
        assert_eq!(
            report.generator.range.start.line, 2,
            "report links the generator definition"
        );
        assert!(
            rename(
                GENERATOR_CONTROLLED_PROGRAM,
                &uri,
                position,
                "solution",
                None
            )
            .is_none(),
            "rename must decline a generator-controlled name"
        );
    }

    /// Round-2 review finding 1: the method name is bound at the
    /// APPLICATION site (`@gen("answer")` — the annotation argument the
    /// handler splices into the extend snippet). In D1 the checked-decl
    /// anchor COINCIDES with the application anchor, so the row-5
    /// "no edits inside generated ranges" guard must not cancel
    /// application-anchored binder edits — they are source text.
    const APPLICATION_BINDER_PROGRAM: &str = r#"
annotation gen(mname: string) on type {
  comptime post(target, ctx) {
    extend (extend_method_literal(target.name, mname, "int", 1))
  }
}

@gen("answer")
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
"#;

    /// Round-2 review finding 1: rename on an application-anchored source
    /// binder edits BOTH the application binder token (line 8) and the call
    /// site (line 12). The pre-fix retain-guard dropped the binder edit
    /// (binder span inside the coincident checked-decl anchor), renaming
    /// only the call sites — after recomputation the generated symbol kept
    /// its old name while every call site used the new one.
    #[test]
    fn application_binder_rename_edits_the_application_token_and_call_sites() {
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let position = call_site_position(APPLICATION_BINDER_PROGRAM);
        let edit = rename(APPLICATION_BINDER_PROGRAM, &uri, position, "solution", None)
            .expect("application-binder rename produces edits");
        let edits = &edit.changes.expect("changes")[&uri];
        let lines: Vec<u32> = edits.iter().map(|e| e.range.start.line).collect();
        assert!(
            lines.contains(&7),
            "the application binder token (`@gen(\"answer\")`) must be \
             edited — dropping it makes the rename partial and corrupting: {lines:?}"
        );
        assert!(lines.contains(&11), "call site edited: {lines:?}");
    }

    /// Rejection row 4 (prepare): prepare-rename declines on a
    /// generator-controlled name; a source-binder name stays renameable.
    #[test]
    fn prepare_rename_declines_generator_controlled_but_allows_source_binder() {
        assert!(
            prepare_rename(
                GENERATOR_CONTROLLED_PROGRAM,
                call_site_position(GENERATOR_CONTROLLED_PROGRAM)
            )
            .is_none(),
            "generator-controlled name must not be offered for rename"
        );
        assert!(
            prepare_rename(
                SOURCE_BINDER_PROGRAM,
                call_site_position(SOURCE_BINDER_PROGRAM)
            )
            .is_some(),
            "source-binder generated name stays renameable"
        );
    }

    #[test]
    fn test_offset_to_line_col() {
        let text = "line1\nline2\nline3";

        assert_eq!(offset_to_line_col(text, 0), (0, 0));
        assert_eq!(offset_to_line_col(text, 3), (0, 3));
        assert_eq!(offset_to_line_col(text, 6), (1, 0));
        assert_eq!(offset_to_line_col(text, 9), (1, 3));
    }

    // W14.2-B1: per-line coverage additions
    #[test]
    fn test_is_identifier_char() {
        assert!(is_identifier_char('a'));
        assert!(is_identifier_char('Z'));
        assert!(is_identifier_char('0'));
        assert!(is_identifier_char('_'));
        assert!(!is_identifier_char(' '));
        assert!(!is_identifier_char('-'));
        assert!(!is_identifier_char('.'));
        assert!(!is_identifier_char('('));
    }

    #[test]
    fn test_is_valid_identifier_invalid_chars() {
        assert!(!is_valid_identifier("foo.bar"));
        assert!(!is_valid_identifier("foo bar"));
        assert!(!is_valid_identifier("foo!"));
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn test_prepare_rename_rejects_keyword() {
        let text = "let myVar = 5";
        let response = prepare_rename(
            text,
            Position {
                line: 0,
                character: 1,
            },
        );
        assert!(response.is_none(), "expected None for keyword 'let'");
    }

    #[test]
    fn test_prepare_rename_rejects_builtin() {
        let text = "let x = abs(5)";
        let response = prepare_rename(
            text,
            Position {
                line: 0,
                character: 9,
            },
        );
        assert!(
            response.is_none(),
            "expected None for builtin 'abs' (got {response:?})"
        );
    }

    #[test]
    fn test_prepare_rename_accepts_user_identifier() {
        let text = "let myVar = 5";
        let response = prepare_rename(
            text,
            Position {
                line: 0,
                character: 5,
            },
        );
        assert!(response.is_some(), "expected Some for user identifier");
    }

    #[test]
    fn test_rename_rejects_invalid_new_name() {
        let text = "let myVar = 5";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let result = rename(
            text,
            &uri,
            Position {
                line: 0,
                character: 5,
            },
            "123bad",
            None,
        );
        assert!(result.is_none(), "expected None for invalid new identifier");
    }

    #[test]
    fn test_rename_rejects_keyword_target() {
        let text = "let myVar = 5";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let result = rename(
            text,
            &uri,
            Position {
                line: 0,
                character: 1,
            },
            "newName",
            None,
        );
        assert!(result.is_none(), "expected None when renaming a keyword");
    }

    #[test]
    fn test_rename_simple_variable() {
        let text = "fn f() {\n  let myVar = 1\n  return myVar + myVar\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let offset = text.find("myVar").unwrap();
        let (line, col) = offset_to_line_col(text, offset);
        let result = rename(
            text,
            &uri,
            Position {
                line,
                character: col,
            },
            "newName",
            None,
        );
        assert!(result.is_some(), "expected rename edits");
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        for te in edits {
            assert_eq!(te.new_text, "newName");
        }
    }

    #[test]
    fn test_find_symbol_occurrences_finds_all() {
        let text = "let foo = 1\nlet bar = foo + foo";
        let edits = find_symbol_occurrences(text, "foo", "baz");
        assert_eq!(edits.len(), 3, "expected 3 occurrences of foo");
        for te in &edits {
            assert_eq!(te.new_text, "baz");
        }
    }

    #[test]
    fn test_find_symbol_occurrences_respects_word_boundary() {
        // "foo" should not match "foobar" or "myfoo"
        let text = "let foo = 1\nlet foobar = 2\nlet myfoo = 3";
        let edits = find_symbol_occurrences(text, "foo", "baz");
        assert_eq!(edits.len(), 1, "expected only the standalone foo match");
    }

    #[test]
    fn test_get_word_range_returns_none_for_non_identifier() {
        let text = "+ + +";
        let result = get_word_range(
            text,
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_none(), "expected None at non-identifier position");
    }

    #[test]
    fn test_get_word_range_returns_span_of_word() {
        let text = "let myVarName = 5";
        let range = get_word_range(
            text,
            Position {
                line: 0,
                character: 7,
            },
        )
        .expect("expected Some range");
        assert_eq!(range.start.character, 4);
        assert_eq!(range.end.character, 13);
    }
}
