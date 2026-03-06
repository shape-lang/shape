//! Code snippet completions

use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};

/// Snippet completions for common patterns (IDE-specific, not from metadata)
pub fn snippet_completions() -> Vec<CompletionItem> {
    vec![
        create_snippet(
            "pattern-def",
            "Pattern Definition",
            r#"pattern ${1:pattern_name} {
    ${2:// Pattern conditions}
    ${3:data[0].close > data[0].open}
}"#,
            "Creates a new pattern definition",
        ),
        create_snippet(
            "function-def",
            "Function Definition",
            r#"fn ${1:function_name}(${2:params}) {
    ${3:// Function body}
    return ${4:result};
}"#,
            "Creates a new function definition",
        ),
        create_snippet(
            "find-query",
            "Find Query",
            r#"find ${1:pattern_name} in ${2:data[0:100]}"#,
            "Creates a find query to search for patterns",
        ),
        create_snippet(
            "if-else",
            "If-Else Statement",
            r#"if ${1:condition} {
    ${2:// true branch}
} else {
    ${3:// false branch}
}"#,
            "Creates an if-else statement",
        ),
        create_snippet(
            "for-loop",
            "For Loop",
            r#"for ${1:item} in ${2:array} {
    ${3:// loop body}
}"#,
            "Creates a for loop",
        ),
        create_snippet(
            "while-loop",
            "While Loop",
            r#"while ${1:condition} {
    ${2:// loop body}
}"#,
            "Creates a while loop",
        ),
        create_snippet(
            "struct-def",
            "Struct Type",
            "type ${1:Name} {\n  ${2:field}: ${3:number}\n}",
            "Define a struct type with typed fields",
        ),
        create_snippet(
            "type-alias",
            "Type Alias",
            "type ${1:Name} = ${2:Type}",
            "Define a type alias",
        ),
        create_snippet(
            "enum-def",
            "Enum",
            "enum ${1:Name} {\n  ${2:Variant1},\n  ${3:Variant2}\n}",
            "Define an enum with variants",
        ),
        create_snippet(
            "meta-def",
            "Meta Definition",
            "meta ${1:TypeName} {\n  format: (v) => ${2:v.toString()}\n}",
            "Define formatting and validation for a type",
        ),
        create_snippet(
            "match-expr",
            "Match Expression",
            "match ${1:value} {\n  ${2:pattern} => ${3:expr}\n  _ => ${4:default}\n}",
            "Pattern match on a value",
        ),
        create_snippet(
            "try-catch",
            "Try-Catch",
            "try {\n  ${1}\n} catch (${2:err}) {\n  ${3}\n}",
            "Handle errors with try-catch",
        ),
        create_snippet(
            "stream-def",
            "Stream",
            "stream ${1:Name} {\n  config {\n    provider: \"${2:source}\"\n  }\n  on_event(${3:event}) {\n    ${4}\n  }\n}",
            "Define a real-time data stream",
        ),
        create_snippet(
            "test-def",
            "Test Suite",
            "test \"${1:suite}\" {\n  test \"${2:case}\" {\n    assert ${3:condition}\n  }\n}",
            "Define a test suite with test cases",
        ),
        create_snippet(
            "use-stmt",
            "Use Module",
            "from ${1:path} use { ${2:name} }",
            "Import module symbols",
        ),
        create_snippet(
            "let-decl",
            "Let Declaration",
            "let ${1:name} = ${2:value}",
            "Declare a variable",
        ),
        create_snippet(
            "const-decl",
            "Const Declaration",
            "const ${1:NAME} = ${2:value}",
            "Declare an immutable constant",
        ),
        create_snippet(
            "annotation-def",
            "Annotation Definition",
            "annotation ${1:name}(${2:param}) {\n  before(fn, args, ctx) {\n    ${3}\n  }\n}",
            "Define a custom annotation",
        ),
        create_snippet(
            "extend-type",
            "Extend Type",
            "extend ${1:Type} {\n  method ${2:name}(${3:params}) {\n    ${4}\n  }\n}",
            "Add methods to an existing type",
        ),
        create_snippet(
            "table-literal",
            "Table Literal",
            "Table { rows: [${1}] }",
            "Create a table literal with typed rows",
        ),
        create_snippet(
            "await-join-all",
            "Await Join All",
            "await join all {\n  ${1:branch1},\n  ${2:branch2}\n}",
            "Await all concurrent branches, returns a tuple of results",
        ),
        create_snippet(
            "await-join-race",
            "Await Join Race",
            "await join race {\n  ${1:branch1},\n  ${2:branch2}\n}",
            "Race concurrent branches, returns the first to complete",
        ),
        create_snippet(
            "async-let",
            "Async Let",
            "async let ${1:name} = ${2:expr}",
            "Spawn an async task and bind the future handle to a variable. Use `await name` to get the result.",
        ),
        create_snippet(
            "async-scope",
            "Async Scope",
            "async scope {\n  ${1:// concurrent tasks}\n}",
            "Structured concurrency boundary. All spawned tasks are cancelled when the scope exits.",
        ),
        create_snippet(
            "for-await",
            "For Await Loop",
            "for await ${1:item} in ${2:stream} {\n  ${3:// process each item}\n}",
            "Iterate over an async stream, awaiting each element.",
        ),
    ]
}

/// Create a snippet completion item
pub fn create_snippet(name: &str, label: &str, snippet: &str, docs: &str) -> CompletionItem {
    CompletionItem {
        label: name.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(label.to_string()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: docs.to_string(),
        })),
        insert_text: Some(snippet.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snippet_completions() {
        let snippets = snippet_completions();
        assert_eq!(
            snippets.len(),
            25,
            "Should have 25 snippets (6 original + 14 new + 2 async join + 3 structured concurrency)"
        );

        let labels: Vec<_> = snippets.iter().map(|s| s.label.as_str()).collect();
        // Original snippets
        assert!(labels.contains(&"pattern-def"));
        assert!(labels.contains(&"function-def"));
        assert!(labels.contains(&"find-query"));
        assert!(labels.contains(&"if-else"));
        assert!(labels.contains(&"for-loop"));
        assert!(labels.contains(&"while-loop"));
        // New snippets
        assert!(labels.contains(&"struct-def"));
        assert!(labels.contains(&"type-alias"));
        assert!(labels.contains(&"enum-def"));
        assert!(labels.contains(&"meta-def"));
        assert!(labels.contains(&"match-expr"));
        assert!(labels.contains(&"try-catch"));
        assert!(labels.contains(&"stream-def"));
        assert!(labels.contains(&"test-def"));
        assert!(labels.contains(&"use-stmt"));
        assert!(labels.contains(&"let-decl"));
        assert!(labels.contains(&"const-decl"));
        assert!(labels.contains(&"annotation-def"));
        assert!(labels.contains(&"extend-type"));
        assert!(labels.contains(&"table-literal"));
    }
}
