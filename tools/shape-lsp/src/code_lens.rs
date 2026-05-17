//! Code lens provider for Shape
//!
//! Provides actionable code lenses for functions, patterns, and tests.

use shape_ast::ast::{Item, Program};
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{CodeLens, Command, Position, Range, Uri};

/// Get code lenses for a document
pub fn get_code_lenses(text: &str, uri: &Uri) -> Vec<CodeLens> {
    let mut lenses = Vec::new();

    // Parse the document, falling back to resilient parser
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return lenses;
            }
            partial.into_program()
        }
    };

    // W2.6 1.55 — build ScopeTree once per document so per-function ref
    // counts use scope-aware resolution (not text-search) — same ref count
    // as `references_provider` produces for the same symbol in-file.
    let tree = crate::scope::ScopeTree::build(&program, text);

    for item in &program.items {
        collect_lenses_for_item(item, &program, &tree, text, uri, &mut lenses);
    }

    lenses
}

/// Resolve a code lens (add the command)
pub fn resolve_code_lens(lens: CodeLens) -> CodeLens {
    // Code lenses are already resolved in get_code_lenses
    lens
}

/// Collect code lenses for an item
fn collect_lenses_for_item(
    item: &Item,
    _program: &Program,
    tree: &crate::scope::ScopeTree,
    text: &str,
    uri: &Uri,
    lenses: &mut Vec<CodeLens>,
) {
    match item {
        Item::Function(func, _) => {
            // Find the line where the function is defined
            if let Some((line, keyword_end_col)) = find_function_line(text, &func.name) {
                // W2.6 1.55 — scope-aware ref count from module scope
                // (top-level fn bindings live there). Falls back to
                // text-search if ScopeTree didn't record this binding
                // (e.g. resilient-parse partial AST).
                let ref_count = count_references_scope_aware(tree, &func.name)
                    .unwrap_or_else(|| count_references(text, &func.name));
                lenses.push(CodeLens {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position { line, character: 0 },
                    },
                    command: Some(Command {
                        title: format!(
                            "{} reference{}",
                            ref_count,
                            if ref_count == 1 { "" } else { "s" }
                        ),
                        command: "shape.findReferences".to_string(),
                        arguments: Some(vec![
                            serde_json::json!(uri.to_string()),
                            serde_json::json!(line),
                            serde_json::json!(keyword_end_col),
                        ]),
                    }),
                    data: None,
                });

                // Add code lenses for annotations on self function
                for annotation in &func.annotations {
                    lenses.push(CodeLens {
                        range: Range {
                            start: Position { line, character: 0 },
                            end: Position { line, character: 0 },
                        },
                        command: Some(Command {
                            title: format!("@{}", annotation.name),
                            command: "shape.showAnnotation".to_string(),
                            arguments: Some(vec![
                                serde_json::json!(uri.to_string()),
                                serde_json::json!(annotation.name),
                                serde_json::json!(func.name),
                            ]),
                        }),
                        data: None,
                    });
                }
            }
        }
        Item::Trait(trait_def, _) => {
            // Add "N implementations" lens on the trait definition
            if let Some(line) = find_trait_line(text, &trait_def.name) {
                let impl_count = count_trait_implementations(text, &trait_def.name);
                lenses.push(CodeLens {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position { line, character: 0 },
                    },
                    command: Some(Command {
                        title: format!(
                            "{} implementation{}",
                            impl_count,
                            if impl_count == 1 { "" } else { "s" }
                        ),
                        command: "shape.findImplementations".to_string(),
                        arguments: Some(vec![
                            serde_json::json!(uri.to_string()),
                            serde_json::json!(trait_def.name),
                        ]),
                    }),
                    data: None,
                });
            }

            // Add per-method lenses showing if the method has a default implementation
            for member in &trait_def.members {
                let (method_name, is_default) = match member {
                    shape_ast::ast::TraitMember::Required(
                        shape_ast::ast::TraitMemberSignature::Method { name, .. },
                    ) => (name.as_str(), false),
                    shape_ast::ast::TraitMember::Default(method_def) => {
                        (method_def.name.as_str(), true)
                    }
                    _ => continue,
                };

                if let Some(method_line) = find_method_in_trait(text, &trait_def.name, method_name)
                {
                    if is_default {
                        lenses.push(CodeLens {
                            range: Range {
                                start: Position {
                                    line: method_line,
                                    character: 0,
                                },
                                end: Position {
                                    line: method_line,
                                    character: 0,
                                },
                            },
                            command: Some(Command {
                                title: "(default)".to_string(),
                                command: "shape.showTraitMethod".to_string(),
                                arguments: Some(vec![
                                    serde_json::json!(uri.to_string()),
                                    serde_json::json!(trait_def.name),
                                    serde_json::json!(method_name),
                                ]),
                            }),
                            data: None,
                        });
                    }
                }
            }
        }
        Item::Test(test, _) => {
            if let Some(line) = find_test_line(text, &test.name) {
                // Run all tests lens
                lenses.push(CodeLens {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position { line, character: 0 },
                    },
                    command: Some(Command {
                        title: "▶ Run All Tests".to_string(),
                        command: "shape.runTests".to_string(),
                        arguments: Some(vec![
                            serde_json::json!(uri.to_string()),
                            serde_json::json!(test.name),
                        ]),
                    }),
                    data: None,
                });

                // Debug tests lens
                lenses.push(CodeLens {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position { line, character: 0 },
                    },
                    command: Some(Command {
                        title: "🐛 Debug Tests".to_string(),
                        command: "shape.debugTests".to_string(),
                        arguments: Some(vec![
                            serde_json::json!(uri.to_string()),
                            serde_json::json!(test.name),
                        ]),
                    }),
                    data: None,
                });
            }
        }
        _ => {}
    }
}

/// Find the line number where a function is defined
fn find_function_line(text: &str, name: &str) -> Option<(u32, u32)> {
    let fn_pattern = format!("fn {}", name);
    let function_pattern = format!("function {}", name);

    for (line_num, line) in text.lines().enumerate() {
        if let Some(col) = line.find(&fn_pattern) {
            return Some((line_num as u32, (col + "fn ".len()) as u32));
        }
        if let Some(col) = line.find(&function_pattern) {
            return Some((line_num as u32, (col + "function ".len()) as u32));
        }
    }
    None
}

/// Find the line number where a test is defined
fn find_test_line(text: &str, name: &str) -> Option<u32> {
    let pattern = format!("test \"{}\"", name);
    for (line_num, line) in text.lines().enumerate() {
        if line.contains(&pattern) {
            return Some(line_num as u32);
        }
    }
    // Also try without quotes
    let pattern = format!("test {}", name);
    for (line_num, line) in text.lines().enumerate() {
        if line.contains(&pattern) {
            return Some(line_num as u32);
        }
    }
    None
}

/// Find the line number where a pattern is defined
#[allow(dead_code)]
fn find_pattern_line(text: &str, name: &str) -> Option<u32> {
    let pattern = format!("pattern {}", name);
    for (line_num, line) in text.lines().enumerate() {
        if line.contains(&pattern) {
            return Some(line_num as u32);
        }
    }
    None
}

/// Find the line number where a trait is defined
fn find_trait_line(text: &str, name: &str) -> Option<u32> {
    let pattern = format!("trait {}", name);
    for (line_num, line) in text.lines().enumerate() {
        if line.trim().starts_with(&pattern) {
            return Some(line_num as u32);
        }
    }
    None
}

/// Count the number of `impl TraitName for ...` blocks in the text
fn count_trait_implementations(text: &str, trait_name: &str) -> usize {
    let pattern = format!("impl {} for", trait_name);
    text.lines()
        .filter(|line| line.trim().starts_with(&pattern) || line.trim().contains(&pattern))
        .count()
}

/// Find the line of a method within a trait definition
fn find_method_in_trait(text: &str, trait_name: &str, method_name: &str) -> Option<u32> {
    let trait_pattern = format!("trait {}", trait_name);
    let mut in_trait = false;
    let mut brace_count: i32 = 0;

    for (line_num, line) in text.lines().enumerate() {
        if line.trim().starts_with(&trait_pattern) {
            in_trait = true;
        }

        if in_trait {
            brace_count += line.matches('{').count() as i32;
            brace_count -= line.matches('}').count() as i32;

            // Check if self line contains the method name
            let trimmed = line.trim();
            if (trimmed.contains(&format!("{}(", method_name))
                || trimmed.starts_with(&format!("method {}(", method_name)))
                && !trimmed.starts_with("trait ")
            {
                return Some(line_num as u32);
            }

            if brace_count == 0 && line.contains('}') {
                in_trait = false;
            }
        }
    }
    None
}

/// W2.6 1.55 — scope-aware reference count for the module-scope binding
/// `name` in `tree`. Returns `None` when no module-scope binding with this
/// name exists (caller falls back to text-search for robustness on
/// resilient-parse partials).
fn count_references_scope_aware(
    tree: &crate::scope::ScopeTree,
    name: &str,
) -> Option<usize> {
    let root = tree.scopes.first()?;
    let mut total: Option<usize> = None;
    for binding in &root.bindings {
        if binding.name == name {
            // ScopeTree::Binding::references excludes the def site itself,
            // matching the legacy text-search semantics (which counted
            // def+uses then subtracted 1).
            let count = binding.references.len();
            total = Some(total.map_or(count, |t| t + count));
        }
    }
    total
}

/// Count references to a symbol in the text
fn count_references(text: &str, name: &str) -> usize {
    let mut count = 0;
    let name_len = name.len();

    for (i, _) in text.match_indices(name) {
        // Check word boundaries
        let before_ok = i == 0 || !text[..i].chars().last().unwrap().is_alphanumeric();
        let after_ok = i + name_len >= text.len()
            || !text[i + name_len..]
                .chars()
                .next()
                .unwrap()
                .is_alphanumeric();

        if before_ok && after_ok {
            count += 1;
        }
    }

    // Subtract 1 for the definition itself
    if count > 0 { count - 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_references() {
        let text = "let foo = 1;\nlet bar = foo + foo;";

        // foo appears 3 times (1 definition + 2 uses)
        // count_references subtracts 1 for definition
        assert_eq!(count_references(text, "foo"), 2);

        // bar appears once (just definition)
        assert_eq!(count_references(text, "bar"), 0);

        // nonexistent
        assert_eq!(count_references(text, "baz"), 0);
    }

    #[test]
    fn test_find_function_line() {
        let text = "// comment\nfunction myFunc() {\n    return 1;\n}";
        assert_eq!(find_function_line(text, "myFunc"), Some((1, 9)));
        let text = "// comment\nfn myFunc() {\n    return 1;\n}";
        assert_eq!(find_function_line(text, "myFunc"), Some((1, 3)));
        assert_eq!(find_function_line(text, "nonexistent"), None);
    }

    #[test]
    fn test_find_trait_line() {
        let text = "// comment\ntrait Queryable {\n    filter(pred): any\n}\n";
        assert_eq!(find_trait_line(text, "Queryable"), Some(1));
        assert_eq!(find_trait_line(text, "NonExistent"), None);
    }

    #[test]
    fn test_count_trait_implementations() {
        let text = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for Table {\n    method filter(pred) { self }\n}\nimpl Queryable for DataFrame {\n    method filter(pred) { self }\n}\n";
        assert_eq!(count_trait_implementations(text, "Queryable"), 2);
        assert_eq!(count_trait_implementations(text, "NonExistent"), 0);
    }

    #[test]
    fn test_trait_code_lens() {
        let text = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for Table {\n    method filter(pred) { self }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let lenses = get_code_lenses(text, &uri);
        // Should have at least one code lens for the trait
        assert!(
            lenses.iter().any(|l| l
                .command
                .as_ref()
                .map_or(false, |c| c.title.contains("implementation"))),
            "Should have implementation count lens for trait. Got: {:?}",
            lenses
                .iter()
                .map(|l| l.command.as_ref().map(|c| c.title.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_count_references_scope_aware_excludes_shadowing() {
        // W2.6 1.55 — ScopeTree-based count must NOT include shadowing inner
        // bindings as references to the outer one. This is the bug the
        // text-search count had: a shadowed inner `let foo` was counted.
        let text = "fn foo() { return 1 }\nfn other() {\n  let foo = 2\n  return foo + foo\n}\nlet x = foo()";
        let program = parse_program(text).unwrap();
        let tree = crate::scope::ScopeTree::build(&program, text);

        let scope_count = count_references_scope_aware(&tree, "foo");
        // Top-level `foo` (the fn) is referenced exactly once: `foo()` at the
        // end. The `let foo = 2` and `foo + foo` inside `other` shadow.
        assert_eq!(
            scope_count,
            Some(1),
            "expected scope-aware count to exclude shadowing inner foo, got {:?}",
            scope_count
        );
    }

    #[test]
    fn test_count_references_scope_aware_lens_integration() {
        // End-to-end: get_code_lenses must produce a "N references" lens
        // whose count matches the scope-aware count.
        let text = "fn helper() { return 1 }\nlet a = helper()\nlet b = helper() + helper()";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let lenses = get_code_lenses(text, &uri);
        let helper_lens = lenses
            .iter()
            .find(|l| {
                l.command
                    .as_ref()
                    .is_some_and(|c| c.title.contains("reference"))
            })
            .expect("should have reference-count lens for helper");
        let title = &helper_lens.command.as_ref().unwrap().title;
        // 3 references: a = helper(), b = helper() + helper()
        assert!(
            title.starts_with("3 references"),
            "expected '3 references' from scope-aware count, got '{}'",
            title
        );
    }

    #[test]
    fn test_find_pattern_line() {
        let text = "// comment\npattern hammer {\n    close > open\n}";
        assert_eq!(find_pattern_line(text, "hammer"), Some(1));
        assert_eq!(find_pattern_line(text, "doji"), None);
    }
}
