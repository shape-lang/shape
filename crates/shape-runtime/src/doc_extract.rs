use shape_ast::ast::{Item, Program};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Kind of documented item
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DocItemKind {
    Function,
    Type,
    Enum,
    Trait,
    Field,
    Variant,
    Constant,
    Module,
}

/// A documented parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocParam {
    pub name: String,
    pub type_name: Option<String>,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

/// A documented item extracted from source code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocItem {
    pub kind: DocItemKind,
    pub name: String,
    pub doc: String,
    pub signature: Option<String>,
    pub type_params: Vec<String>,
    pub params: Vec<DocParam>,
    pub return_type: Option<String>,
    pub children: Vec<DocItem>,
}

/// Documentation for an entire package
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageDocs {
    pub readme: Option<String>,
    pub modules: HashMap<String, Vec<DocItem>>,
}

/// Extract doc comments from source text at a given byte position.
///
/// Walks backward from the position to collect consecutive `///` comment lines.
/// This is extracted from stdlib_metadata.rs extract_doc_comment for shared use.
pub fn extract_doc_comment_at(source: &str, byte_pos: usize) -> Option<String> {
    let clamped = byte_pos.min(source.len());
    let before = &source[..clamped];
    let line_index = before.matches('\n').count();

    if line_index == 0 {
        return None;
    }

    let lines: Vec<&str> = source.lines().collect();
    let mut docs = Vec::new();
    let mut i = line_index.saturating_sub(1);

    loop {
        let trimmed = lines.get(i).map(|l| l.trim()).unwrap_or("");
        if let Some(comment) = trimmed.strip_prefix("///") {
            docs.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
        } else if docs.is_empty() {
            // Allow blank lines between item and doc comment
            if !trimmed.is_empty() {
                break;
            }
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }

    if docs.is_empty() {
        None
    } else {
        docs.reverse();
        Some(docs.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_doc_comment_at tests ---

    #[test]
    fn test_single_line_doc_comment() {
        let source = "/// Hello\nfn greet() { }";
        let byte_pos = source.find("fn greet").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, Some("Hello".to_string()));
    }

    #[test]
    fn test_multi_line_doc_comment() {
        let source = "/// Line one\n/// Line two\n/// Line three\nfn multi() { }";
        let byte_pos = source.find("fn multi").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, Some("Line one\nLine two\nLine three".to_string()));
    }

    #[test]
    fn test_no_doc_comment() {
        let source = "let x = 5\nfn no_doc() { }";
        let byte_pos = source.find("fn no_doc").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, None);
    }

    #[test]
    fn test_blank_line_between_comment_and_item() {
        let source = "/// Docs here\n\nfn spaced() { }";
        let byte_pos = source.find("fn spaced").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, Some("Docs here".to_string()));
    }

    #[test]
    fn test_doc_comment_at_file_start_line_zero() {
        // Item is on line 0 (no newline before it) → line_index == 0 → returns None
        let source = "fn at_start() { }";
        let result = extract_doc_comment_at(source, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_empty_source_string() {
        let result = extract_doc_comment_at("", 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_doc_comment_without_space_after_slashes() {
        let source = "///NoSpace\nfn tight() { }";
        let byte_pos = source.find("fn tight").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, Some("NoSpace".to_string()));
    }

    #[test]
    fn test_regular_comment_not_doc() {
        let source = "// Regular comment\nfn regular() { }";
        let byte_pos = source.find("fn regular").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        assert_eq!(result, None);
    }

    #[test]
    fn test_multiple_blank_lines_break_collection() {
        // After first blank line the doc comment is collected; the second blank line
        // hits "not empty and docs not empty" so only the doc above is kept.
        let source = "/// First\n\n\nfn far() { }";
        let byte_pos = source.find("fn far").unwrap();
        let result = extract_doc_comment_at(source, byte_pos);
        // The function walks backwards: line 3 = "fn far", line 2 = "", line 1 = "", line 0 = "/// First"
        // At line 2 (first blank from item), docs is empty so it skips.
        // At line 1 (second blank), docs is still empty so it skips again.
        // At line 0, it finds the doc comment.
        assert_eq!(result, Some("First".to_string()));
    }

    #[test]
    fn test_byte_pos_clamped_beyond_source() {
        let source = "/// Over\nfn over() { }";
        // byte_pos far beyond source length → clamped
        let result = extract_doc_comment_at(source, 99999);
        // line_index will be 1 (there is one newline), so it looks at line 0
        assert_eq!(result, Some("Over".to_string()));
    }

    // --- extract_docs_from_ast tests ---

    #[test]
    fn test_extract_docs_function_from_ast() {
        let source = "/// Doc for hello\nfn hello() { }";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].name, "hello");
        assert_eq!(docs[0].kind, DocItemKind::Function);
        assert_eq!(docs[0].doc, "Doc for hello");
    }

    #[test]
    fn test_extract_docs_struct_from_ast() {
        let source = "/// A point\ntype Point { x: int, y: int }";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].name, "Point");
        assert_eq!(docs[0].kind, DocItemKind::Type);
        assert_eq!(docs[0].doc, "A point");
        assert_eq!(docs[0].children.len(), 2);
    }

    #[test]
    fn test_extract_docs_no_comments() {
        let source = "fn bare() { }";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].doc, ""); // no doc comment → empty string
    }

    #[test]
    fn test_extract_docs_empty_source() {
        let source = "";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert!(docs.is_empty());
    }

    #[test]
    fn test_extract_docs_multi_line_on_function() {
        let source = "/// First line\n/// Second line\nfn documented() { }";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].doc, "First line\nSecond line");
    }
}

/// Format a TypeAnnotation as a human-readable string.
fn format_type_annotation(ta: &shape_ast::ast::TypeAnnotation) -> String {
    use shape_ast::ast::TypeAnnotation;
    match ta {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Array(inner) => format!("Array<{}>", format_type_annotation(inner)),
        TypeAnnotation::Tuple(items) => {
            let parts: Vec<String> = items.iter().map(format_type_annotation).collect();
            format!("({})", parts.join(", "))
        }
        TypeAnnotation::Optional(inner) => format!("{}?", format_type_annotation(inner)),
        TypeAnnotation::Generic { name, args } => {
            let parts: Vec<String> = args.iter().map(format_type_annotation).collect();
            format!("{}<{}>", name, parts.join(", "))
        }
        TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Function { params, returns } => {
            let parts: Vec<String> = params
                .iter()
                .map(|p| format_type_annotation(&p.type_annotation))
                .collect();
            format!("({}) => {}", parts.join(", "), format_type_annotation(returns))
        }
        TypeAnnotation::Union(items) => {
            let parts: Vec<String> = items.iter().map(format_type_annotation).collect();
            parts.join(" | ")
        }
        _ => "...".to_string(),
    }
}

/// Format type params as a string like `<T, U: Display>`.
fn format_type_params(type_params: &Option<Vec<shape_ast::ast::TypeParam>>) -> Vec<String> {
    match type_params {
        Some(params) => params.iter().map(|tp| tp.name.clone()).collect(),
        None => Vec::new(),
    }
}

/// Extract doc items from a function definition.
fn extract_function_doc(
    source: &str,
    func: &shape_ast::ast::FunctionDef,
    span: &shape_ast::ast::Span,
) -> DocItem {
    let doc = extract_doc_comment_at(source, span.start).unwrap_or_default();

    let params: Vec<DocParam> = func
        .params
        .iter()
        .map(|p| DocParam {
            name: p.simple_name().unwrap_or("_").to_string(),
            type_name: p.type_annotation.as_ref().map(format_type_annotation),
            description: None,
            default_value: None,
        })
        .collect();

    let type_params = format_type_params(&func.type_params);

    let return_type = func.return_type.as_ref().map(format_type_annotation);

    // Build signature string
    let tp_str = if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    };
    let param_strs: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let name = p.simple_name().unwrap_or("_");
            match &p.type_annotation {
                Some(ta) => format!("{}: {}", name, format_type_annotation(ta)),
                None => name.to_string(),
            }
        })
        .collect();
    let ret_str = match &func.return_type {
        Some(ta) => format!(" -> {}", format_type_annotation(ta)),
        None => String::new(),
    };
    let signature = Some(format!(
        "fn {}{}({}){}",
        func.name,
        tp_str,
        param_strs.join(", "),
        ret_str
    ));

    DocItem {
        kind: DocItemKind::Function,
        name: func.name.clone(),
        doc,
        signature,
        type_params,
        params,
        return_type,
        children: Vec::new(),
    }
}

/// Extract doc items from a struct type definition.
fn extract_struct_doc(
    source: &str,
    st: &shape_ast::ast::StructTypeDef,
    span: &shape_ast::ast::Span,
) -> DocItem {
    let doc = extract_doc_comment_at(source, span.start).unwrap_or_default();
    let type_params = format_type_params(&st.type_params);

    let children: Vec<DocItem> = st
        .fields
        .iter()
        .map(|f| DocItem {
            kind: DocItemKind::Field,
            name: f.name.clone(),
            doc: String::new(),
            signature: Some(format!(
                "{}: {}",
                f.name,
                format_type_annotation(&f.type_annotation)
            )),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(format_type_annotation(&f.type_annotation)),
            children: Vec::new(),
        })
        .collect();

    DocItem {
        kind: DocItemKind::Type,
        name: st.name.clone(),
        doc,
        signature: None,
        type_params,
        params: Vec::new(),
        return_type: None,
        children,
    }
}

/// Extract doc items from an enum definition.
fn extract_enum_doc(
    source: &str,
    en: &shape_ast::ast::EnumDef,
    span: &shape_ast::ast::Span,
) -> DocItem {
    let doc = extract_doc_comment_at(source, span.start).unwrap_or_default();
    let type_params = format_type_params(&en.type_params);

    let children: Vec<DocItem> = en
        .members
        .iter()
        .map(|m| DocItem {
            kind: DocItemKind::Variant,
            name: m.name.clone(),
            doc: String::new(),
            signature: None,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            children: Vec::new(),
        })
        .collect();

    DocItem {
        kind: DocItemKind::Enum,
        name: en.name.clone(),
        doc,
        signature: None,
        type_params,
        params: Vec::new(),
        return_type: None,
        children,
    }
}

/// Extract doc items from a trait definition.
fn extract_trait_doc(
    source: &str,
    tr: &shape_ast::ast::TraitDef,
    span: &shape_ast::ast::Span,
) -> DocItem {
    let doc = extract_doc_comment_at(source, span.start).unwrap_or_default();
    let type_params = format_type_params(&tr.type_params);

    DocItem {
        kind: DocItemKind::Trait,
        name: tr.name.clone(),
        doc,
        signature: None,
        type_params,
        params: Vec::new(),
        return_type: None,
        children: Vec::new(),
    }
}

/// Extract documentation items from a parsed AST and its source text.
///
/// Walks top-level items (functions, types, enums, traits) and extracts
/// `///` doc comments attached to each item.
pub fn extract_docs_from_ast(source: &str, ast: &Program) -> Vec<DocItem> {
    let mut docs = Vec::new();

    for item in &ast.items {
        match item {
            Item::Function(func, span) => {
                docs.push(extract_function_doc(source, func, span));
            }
            Item::Export(export, span) => {
                use shape_ast::ast::ExportItem;
                match &export.item {
                    ExportItem::Function(func) => {
                        docs.push(extract_function_doc(source, func, span));
                    }
                    ExportItem::Struct(st) => {
                        docs.push(extract_struct_doc(source, st, span));
                    }
                    ExportItem::Enum(en) => {
                        docs.push(extract_enum_doc(source, en, span));
                    }
                    ExportItem::Trait(tr) => {
                        docs.push(extract_trait_doc(source, tr, span));
                    }
                    _ => {}
                }
            }
            Item::StructType(st, span) => {
                docs.push(extract_struct_doc(source, st, span));
            }
            Item::Enum(en, span) => {
                docs.push(extract_enum_doc(source, en, span));
            }
            Item::Trait(tr, span) => {
                docs.push(extract_trait_doc(source, tr, span));
            }
            _ => {}
        }
    }

    docs
}
