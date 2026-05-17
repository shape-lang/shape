//! Go-to-definition and find references provider
//!
//! Enables navigation to symbol definitions and finding all references.

use crate::annotation_discovery::AnnotationDiscovery;
use crate::document::DocumentManager;
use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, offset_to_line_col, position_to_offset};
use shape_ast::ast::{ImportItems, Item, Program, Span, Statement};
use shape_ast::parser::parse_program;
use std::path::{Path, PathBuf};
use tower_lsp_server::ls_types::{GotoDefinitionResponse, Location, Position, Range, Uri};

/// Find the definition of a symbol at the given position.
///
/// When `cached_program` is provided, it is used as a fallback AST when
/// the current source text fails to parse.
pub fn get_definition(
    text: &str,
    position: Position,
    uri: &Uri,
    module_cache: Option<&ModuleCache>,
    annotation_discovery: Option<&AnnotationDiscovery>,
    cached_program: Option<&Program>,
) -> Option<GotoDefinitionResponse> {
    // Get the word at cursor
    let word = get_word_at_position(text, position)?;

    // Parse the current file, falling back to cached program or resilient parser
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            if let Some(cached) = cached_program {
                cached.clone()
            } else {
                // Fall back to resilient parser — always succeeds with partial results
                let partial = shape_ast::parser::resilient::parse_program_resilient(text);
                if partial.items.is_empty() {
                    return None;
                }
                partial.into_program()
            }
        }
    };

    // First, try to find definition in the current file
    if let Some(location) = find_definition_location(&program, &word, uri, text) {
        return Some(GotoDefinitionResponse::Scalar(location));
    }

    // If not found locally, check if it's an imported symbol
    if let Some(cache) = module_cache {
        if let Some(location) = find_imported_definition(&program, &word, uri, cache) {
            return Some(GotoDefinitionResponse::Scalar(location));
        }
    }

    // Check if it's an annotation
    if let Some(discovery) = annotation_discovery {
        if let Some(location) = find_annotation_definition(&word, discovery, uri) {
            return Some(GotoDefinitionResponse::Scalar(location));
        }
    }

    None
}

/// Find all references to a symbol at the given position.
///
/// Uses scope-aware resolution via `ScopeTree` to correctly handle
/// variable shadowing and lexical scoping.
pub fn get_references(text: &str, position: Position, uri: &Uri) -> Option<Vec<Location>> {
    get_references_with_fallback(text, position, uri, None)
}

/// W2.6 — Find all references to a symbol at the given position, including
/// cross-file references from open documents and workspace `.shape` files.
///
/// Algorithm:
///  1. Resolve the symbol locally via `ScopeTree::references_of` (same as
///     `get_references_with_fallback`).
///  2. If the local binding is module-scope-visible (top-level item — fn /
///     type / trait / enum / pub var), enumerate other files (open docs +
///     workspace `.shape` files via `ModuleCache::enumerate_workspace_shape_files`)
///     and scan each one's `ScopeTree` for **module-scope** references to the
///     same name. Lexical scoping within each file is preserved by
///     `ScopeTree::references_of` — only top-level usages cross.
///  3. Locally-scoped bindings (loop vars, closure params, block lets) do not
///     produce cross-file results.
///
/// Returns `None` if no references are found anywhere.
pub fn get_references_cross_file(
    text: &str,
    position: Position,
    uri: &Uri,
    cached_program: Option<&Program>,
    documents: Option<&DocumentManager>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&Path>,
) -> Option<Vec<Location>> {
    // Local file references via ScopeTree (and text-search fallback).
    let mut locations = get_references_with_fallback(text, position, uri, cached_program)
        .unwrap_or_default();

    // Determine the symbol name + whether it's module-scope-visible.
    let Some(word) = get_word_at_position(text, position) else {
        return if locations.is_empty() {
            None
        } else {
            Some(locations)
        };
    };

    // Parse to inspect the binding kind. If parse fails, skip cross-file.
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => match cached_program {
            Some(p) => p.clone(),
            None => {
                return if locations.is_empty() {
                    None
                } else {
                    Some(locations)
                };
            }
        },
    };

    if !is_module_scope_symbol(&program, &word) {
        // Local-scope binding (loop var, closure param, block let) — no
        // cross-file lookup. Return whatever local refs we found.
        return if locations.is_empty() {
            None
        } else {
            Some(locations)
        };
    }

    // Cross-file scan: open documents (other than current uri) + workspace
    // .shape files (de-duplicated).
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
            collect_module_scope_refs_in_file(
                &other_text,
                &other_uri,
                &word,
                &mut locations,
            );
        }
    }

    if let (Some(cache), Some(root)) = (module_cache, workspace_root) {
        let _ = cache; // Reserved for future symbol-identity refinement.
        for path in cache.enumerate_workspace_shape_files(root) {
            if !visited.insert(path.clone()) {
                continue;
            }
            let Some(other_uri) = Uri::from_file_path(&path) else {
                continue;
            };
            // Skip files already visited via open documents.
            let Ok(other_text) = std::fs::read_to_string(&path) else {
                continue;
            };
            collect_module_scope_refs_in_file(
                &other_text,
                &other_uri,
                &word,
                &mut locations,
            );
        }
    }

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// Return true when `name` is bound at module (top-level) scope in
/// `program` — i.e. it is a candidate for cross-file references because
/// other files could import or refer to it.
///
/// W2.6 conservative heuristic: top-level fn / type / trait / enum /
/// struct / foreign-fn / module-level variable declarations are
/// module-scope-visible. Statement-nested `let`, loop vars, closure
/// params are not.
fn is_module_scope_symbol(program: &Program, name: &str) -> bool {
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
                ImportItems::Namespace { name: ns_name, alias } => {
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

/// Collect ScopeTree references to a top-level `name` in `text`, appending
/// `Location`s into `out`. Only the module-scope binding (and its
/// references) is considered — locally-shadowing inner bindings are
/// excluded by `ScopeTree::references_of` semantics.
fn collect_module_scope_refs_in_file(
    text: &str,
    uri: &Uri,
    name: &str,
    out: &mut Vec<Location>,
) {
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return;
            }
            partial.into_program()
        }
    };

    if !is_module_scope_symbol(&program, name) {
        return;
    }

    let tree = crate::scope::ScopeTree::build(&program, text);
    // Find any module-scope binding with this name. ScopeTree's first
    // scope is the module (root) scope; its bindings are the top-level
    // names.
    let Some(root) = tree.scopes.first() else {
        return;
    };
    for binding in &root.bindings {
        if binding.name != name {
            continue;
        }
        let push = |span: (usize, usize), out: &mut Vec<Location>| {
            let (sl, sc) = offset_to_line_col(text, span.0);
            let (el, ec) = offset_to_line_col(text, span.1);
            out.push(Location {
                uri: uri.clone(),
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
            });
        };
        push(binding.def_span, out);
        for span in &binding.references {
            push(*span, out);
        }
    }
}

/// Find all references to a symbol at the given position, with cached program fallback.
pub fn get_references_with_fallback(
    text: &str,
    position: Position,
    uri: &Uri,
    cached_program: Option<&Program>,
) -> Option<Vec<Location>> {
    // Get the byte offset of the cursor
    let offset = position_to_offset(text, position)?;

    // Parse, falling back to cached program or resilient parser
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            if let Some(cached) = cached_program {
                cached.clone()
            } else {
                let partial = shape_ast::parse_program_resilient(text);
                if partial.items.is_empty() {
                    return None;
                }
                partial.into_program()
            }
        }
    };
    let tree = crate::scope::ScopeTree::build(&program, text);

    // Find all references (def + uses) via scope-aware resolution
    let spans = tree.references_of(offset)?;

    let locations: Vec<Location> = spans
        .into_iter()
        .map(|(start, end)| {
            let (start_line, start_col) = offset_to_line_col(text, start);
            let (end_line, end_col) = offset_to_line_col(text, end);
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
        })
        .collect();

    if locations.is_empty() {
        // Fallback to text-based search if scope tree didn't find anything
        let word = get_word_at_position(text, position)?;
        let fallback = find_all_references(&program, &word, uri, text);
        if fallback.is_empty() {
            None
        } else {
            Some(fallback)
        }
    } else {
        Some(locations)
    }
}

/// Find the location where a symbol is defined
fn find_definition_location(
    program: &Program,
    symbol_name: &str,
    uri: &Uri,
    text: &str,
) -> Option<Location> {
    for item in &program.items {
        match item {
            Item::Function(func, _) if func.name == symbol_name => {
                return Some(create_location_from_span(uri, func.name_span, text));
            }
            Item::VariableDecl(var_decl, item_span) => {
                for (name, name_span) in crate::symbols::get_pattern_names(&var_decl.pattern) {
                    if name == symbol_name {
                        let span = if name_span.is_dummy() {
                            *item_span
                        } else {
                            name_span
                        };
                        return Some(create_location_from_span(uri, span, text));
                    }
                }
            }
            Item::Statement(Statement::VariableDecl(var_decl, stmt_span), _) => {
                for (name, name_span) in crate::symbols::get_pattern_names(&var_decl.pattern) {
                    if name == symbol_name {
                        let span = if name_span.is_dummy() {
                            *stmt_span
                        } else {
                            name_span
                        };
                        return Some(create_location_from_span(uri, span, text));
                    }
                }
            }
            Item::TypeAlias(type_alias, item_span) if type_alias.name == symbol_name => {
                return Some(create_location_from_span(uri, *item_span, text));
            }
            Item::Enum(enum_def, item_span) if enum_def.name == symbol_name => {
                return Some(create_location_from_span(uri, *item_span, text));
            }
            Item::Trait(trait_def, item_span) if trait_def.name == symbol_name => {
                return Some(create_location_from_span(uri, *item_span, text));
            }
            Item::Impl(impl_block, _) => {
                // Navigate from impl method name to trait definition
                let trait_name_str = match &impl_block.trait_name {
                    shape_ast::ast::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
                };
                // If clicking on the trait name in `impl TraitName for Type`,
                // navigate to the trait definition
                if trait_name_str == symbol_name {
                    // Find the trait definition elsewhere in the program
                    for other_item in &program.items {
                        if let Item::Trait(td, ts) = other_item {
                            if td.name == symbol_name {
                                return Some(create_location_from_span(uri, *ts, text));
                            }
                        }
                    }
                }
                // If clicking on a method name inside the impl block,
                // navigate to the trait's method signature
                for method in &impl_block.methods {
                    if method.name == symbol_name {
                        // Find the trait definition and navigate to the method member
                        for other_item in &program.items {
                            if let Item::Trait(td, ts) = other_item {
                                if td.name == trait_name_str {
                                    // Return trait span (method-level spans not yet available)
                                    return Some(create_location_from_span(uri, *ts, text));
                                }
                            }
                        }
                    }
                }
            }
            Item::Extend(extend_stmt, item_span) => {
                // Navigate from method name usage to method definition in extend block
                for method in &extend_stmt.methods {
                    if method.name == symbol_name {
                        return Some(create_location_from_span(uri, *item_span, text));
                    }
                }
            }
            Item::StructType(struct_def, item_span) if struct_def.name == symbol_name => {
                return Some(create_location_from_span(uri, *item_span, text));
            }
            _ => {}
        }
    }

    // Sprint 4L: If the symbol is "format" and it's a method call on a typed object,
    // try to navigate to the Display trait impl for that type.
    if symbol_name == "format" || symbol_name == "toString" {
        // Find impl Display for ... blocks
        for item in &program.items {
            if let Item::Impl(impl_block, item_span) = item {
                let trait_name_str = match &impl_block.trait_name {
                    shape_ast::ast::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
                };
                if trait_name_str == "Display" {
                    // Check if the impl block has a method matching our symbol
                    for method in &impl_block.methods {
                        if method.name == symbol_name {
                            return Some(create_location_from_span(uri, *item_span, text));
                        }
                    }
                    // Even if method not found, navigate to impl block
                    return Some(create_location_from_span(uri, *item_span, text));
                }
            }
        }
    }

    None
}

/// Find all references to a symbol in the program
fn find_all_references(
    _program: &Program,
    symbol_name: &str,
    uri: &Uri,
    text: &str,
) -> Vec<Location> {
    let mut locations = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    // Simple approach: find all occurrences of the symbol name in the text
    // A more sophisticated approach would parse the AST and find identifier references
    for (line_idx, line) in lines.iter().enumerate() {
        let mut char_pos = 0;
        while let Some(pos) = line[char_pos..].find(symbol_name) {
            let absolute_pos = char_pos + pos;

            // Check if it's a word boundary (not part of another identifier)
            let is_start_boundary = absolute_pos == 0
                || !line
                    .chars()
                    .nth(absolute_pos - 1)
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);

            let is_end_boundary = absolute_pos + symbol_name.len() >= line.len()
                || !line
                    .chars()
                    .nth(absolute_pos + symbol_name.len())
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);

            if is_start_boundary && is_end_boundary {
                locations.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_idx as u32,
                            character: absolute_pos as u32,
                        },
                        end: Position {
                            line: line_idx as u32,
                            character: (absolute_pos + symbol_name.len()) as u32,
                        },
                    },
                });
            }

            char_pos = absolute_pos + symbol_name.len();
        }
    }

    locations
}

/// Create a location from a span
fn create_location_from_span(uri: &Uri, span: Span, text: &str) -> Location {
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

/// Find the definition of a symbol in imported modules
fn find_imported_definition(
    program: &Program,
    symbol_name: &str,
    current_uri: &Uri,
    module_cache: &ModuleCache,
) -> Option<Location> {
    // Get current file path from URI
    let current_path = current_uri.to_file_path()?.into_owned();

    // Look through import statements to find where the symbol comes from
    for item in &program.items {
        if let Item::Import(import_stmt, _span) = item {
            // Check if this import includes the symbol we're looking for
            let imports_symbol = match &import_stmt.items {
                ImportItems::Named(specs) => specs.iter().any(|spec| {
                    let imported_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    imported_name == symbol_name
                }),
                ImportItems::Namespace { name, alias } => {
                    let local_name = alias.as_ref().unwrap_or(name);
                    local_name == symbol_name
                }
            };

            if !imports_symbol {
                continue;
            }

            // Resolve the import path
            // Note: module resolution uses dot-separated paths.
            // std. imports resolve via stdlib path.
            let resolved_path =
                module_cache.resolve_import(&import_stmt.from, &current_path, None)?;

            // Load the module
            let module_info =
                module_cache.load_module_with_context(&resolved_path, &current_path, None)?;

            // Find the symbol in the module's exports
            for export in &module_info.exports {
                if export.exported_name() == symbol_name {
                    // Find the actual definition in the module's program
                    let target_uri = Uri::from_file_path(&module_info.path)?;
                    let source = std::fs::read_to_string(&module_info.path).ok()?;

                    // Look up the definition in the target module
                    let location = find_definition_location(
                        &module_info.program,
                        &export.name,
                        &target_uri,
                        &source,
                    )?;

                    return Some(location);
                }
            }
        }
    }

    None
}

/// Find the definition of an annotation
fn find_annotation_definition(
    annotation_name: &str,
    annotation_discovery: &AnnotationDiscovery,
    current_uri: &Uri,
) -> Option<Location> {
    // Get annotation info
    let info = annotation_discovery.get(annotation_name)?;

    // If the annotation has a valid location (not a built-in), create a location
    if info.location != Span::default() {
        // Determine which file the annotation is defined in
        let target_uri = if let Some(ref source_path) = info.source_file {
            // Imported annotation — navigate to the source file
            Uri::from_file_path(source_path)?
        } else {
            // Local annotation — same file
            current_uri.clone()
        };

        // Read source to convert byte offset to line/col
        let source = if let Some(ref source_path) = info.source_file {
            std::fs::read_to_string(source_path).ok()?
        } else {
            // For local annotations, we'd need the current file text
            // but we don't have it here. Return a reasonable position.
            return Some(Location {
                uri: target_uri,
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
            });
        };

        let (line, col) = offset_to_line_col(&source, info.location.start);
        Some(Location {
            uri: target_uri,
            range: Range {
                start: Position {
                    line,
                    character: col,
                },
                end: Position {
                    line,
                    character: col + annotation_name.len() as u32,
                },
            },
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_word_at_position() {
        let text = "let myVar = 5;";
        let word = get_word_at_position(
            text,
            Position {
                line: 0,
                character: 5,
            },
        );
        assert_eq!(word, Some("myVar".to_string()));
    }

    #[test]
    fn test_find_function_definition() {
        let code = r#"function myFunc(x, y) {
    return x + y;
}

let result = myFunc(1, 2);
"#;
        let program = parse_program(code).unwrap();
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let location = find_definition_location(&program, "myFunc", &uri, code);
        assert!(location.is_some());

        let loc = location.unwrap();
        assert_eq!(loc.range.start.line, 0);
        // Should point to the function name "myFunc"
        assert_eq!(loc.range.start.character, 9); // "function " is 9 chars
    }

    #[test]
    fn test_find_variable_definition() {
        let code = r#"let myVar = 42;
let x = myVar + 5;
"#;
        let program = parse_program(code).unwrap();
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let location = find_definition_location(&program, "myVar", &uri, code);
        assert!(location.is_some());
    }

    #[test]
    fn test_find_references() {
        let code = r#"let myVar = 42;
let x = myVar + 5;
let y = myVar * 2;
"#;
        let program = parse_program(code).unwrap();
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let refs = find_all_references(&program, "myVar", &uri, code);
        assert_eq!(refs.len(), 3); // Definition + 2 usages
    }

    #[test]
    fn test_get_definition_with_module_cache() {
        let code = r#"function localFunc() {
    return 42;
}
"#;
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let cache = ModuleCache::new();

        let definition = get_definition(
            code,
            Position {
                line: 0,
                character: 10,
            },
            &uri,
            Some(&cache),
            None,
            None,
        );
        assert!(definition.is_some());
    }

    #[test]
    fn test_find_imported_definition_not_found() {
        let code = r#"from utils use { foo };

let x = foo();
"#;
        let program = parse_program(code).unwrap();
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let cache = ModuleCache::new();

        // This will return None because the module doesn't exist
        let location = find_imported_definition(&program, "foo", &uri, &cache);
        assert!(location.is_none());
    }

    #[test]
    fn test_is_module_scope_symbol_top_level_fn() {
        let code = "fn foo() { return 1 }\nlet x = foo()";
        let program = parse_program(code).unwrap();
        assert!(is_module_scope_symbol(&program, "foo"));
        assert!(is_module_scope_symbol(&program, "x"));
        // Inner locals are not module-scope-visible.
        assert!(!is_module_scope_symbol(&program, "nope"));
    }

    #[test]
    fn test_collect_module_scope_refs_finds_call_site() {
        let text = "fn helper() { return 1 }\nlet x = helper() + helper()";
        let uri = Uri::from_file_path("/other.shape").unwrap();
        let mut out = Vec::new();
        collect_module_scope_refs_in_file(text, &uri, "helper", &mut out);
        // def + 2 references = 3 locations
        assert!(
            out.len() >= 3,
            "expected def + at least 2 refs to `helper`, got {}",
            out.len()
        );
    }

    #[test]
    fn test_get_references_cross_file_module_scope() {
        // Simulate two open documents that both reference `shared`.
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text = "fn shared() { return 1 }\nlet a = shared()".to_string();
        let other_text = "fn shared() { return 2 }\nlet b = shared() + shared()".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        // Click on `shared` in main.shape (col 3 of `fn shared()`)
        let pos = Position {
            line: 0,
            character: 3,
        };
        let refs = get_references_cross_file(
            &main_text,
            pos,
            &main_uri,
            None,
            Some(&docs),
            None,
            None,
        )
        .expect("should find cross-file references");
        // Local def + local 1 use + other def + other 2 uses = 5
        assert!(
            refs.len() >= 4,
            "expected cross-file refs, got {}: {:?}",
            refs.len(),
            refs
        );
        assert!(
            refs.iter().any(|loc| &loc.uri == &other_uri),
            "expected at least one reference from /other.shape"
        );
    }

    #[test]
    fn test_get_references_cross_file_local_binding_no_crossover() {
        // Local `let` inside a function — should NOT cascade to other files.
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text =
            "fn outer() {\n  let local = 1\n  return local + local\n}".to_string();
        let other_text =
            "fn other() {\n  let local = 5\n  return local\n}".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        // Click on `local` in main.shape at its definition site
        let local_offset = main_text.find("local").unwrap();
        let (line, col) = offset_to_line_col(&main_text, local_offset);
        let pos = Position {
            line,
            character: col,
        };
        let refs = get_references_cross_file(
            &main_text,
            pos,
            &main_uri,
            None,
            Some(&docs),
            None,
            None,
        );
        // Expect refs only in main.shape (local-scope binding)
        if let Some(refs) = refs {
            assert!(
                refs.iter().all(|loc| &loc.uri == &main_uri),
                "local-scope `local` should NOT cascade to other files, got: {:?}",
                refs
            );
        }
    }

    #[test]
    fn test_references_with_broken_code() {
        // Code with a syntax error after a valid function
        let code =
            "fn greet(name) {\n  return name\n}\nlet x = greet(\"hi\")\n??broken syntax here";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        // Position on "greet" in the function definition (line 0, char 3)
        let refs = get_references_with_fallback(
            code,
            Position {
                line: 0,
                character: 3,
            },
            &uri,
            None, // no cached program, relies on resilient parser
        );

        // With resilient parsing, we should find at least the definition of "greet"
        assert!(
            refs.is_some(),
            "Should find references even with broken code via resilient parsing"
        );
    }
}
