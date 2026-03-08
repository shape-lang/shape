use crate::ast::{
    DocComment, DocEntry, DocLink, DocTag, DocTagKind, DocTarget, DocTargetKind, ExportItem, Item,
    Program, ProgramDocs, Span, TraitMember, TypeParam,
};
use pest::iterators::Pair;

use super::Rule;

pub fn parse_doc_comment(pair: Pair<Rule>) -> DocComment {
    debug_assert_eq!(pair.as_rule(), Rule::doc_comment);
    let span = crate::parser::pair_span(&pair);

    let lines = pair
        .into_inner()
        .filter(|line| line.as_rule() == Rule::doc_comment_line)
        .map(parse_doc_line)
        .collect::<Vec<_>>();

    parse_doc_lines(span, &lines)
}

pub fn build_program_docs(program: &Program) -> ProgramDocs {
    let mut collector = DocCollector::default();
    collector.collect_items(&program.items, &[]);
    ProgramDocs {
        entries: collector.entries,
    }
}

#[derive(Debug, Clone)]
struct DocLine {
    text: String,
    span: Span,
}

fn parse_doc_line(line: Pair<Rule>) -> DocLine {
    let raw = line.as_str();
    let raw_span = crate::parser::pair_span(&line);
    let rest = raw
        .strip_prefix("///")
        .expect("doc comment lines must start with ///");
    let prefix_len = if rest.starts_with(' ') { 4 } else { 3 };
    let text = rest.strip_prefix(' ').unwrap_or(rest).to_string();
    let content_start = (raw_span.start + prefix_len).min(raw_span.end);
    DocLine {
        text,
        span: Span::new(content_start, raw_span.end),
    }
}

fn parse_doc_lines(span: Span, lines: &[DocLine]) -> DocComment {
    let mut body_lines = Vec::new();
    let mut tags = Vec::new();
    let mut current_tag: Option<DocTag> = None;

    for line in lines {
        let trimmed = line.text.trim_end();
        if let Some(parsed_tag) = parse_tag_line(line) {
            if let Some(tag) = current_tag.take() {
                tags.push(tag);
            }
            current_tag = Some(parsed_tag);
            continue;
        }

        if let Some(tag) = current_tag.as_mut() {
            if !tag.body.is_empty() {
                tag.body.push('\n');
            }
            tag.body.push_str(trimmed);
            tag.span = tag.span.merge(line.span);
            tag.body_span = Some(match tag.body_span {
                Some(body_span) => body_span.merge(line.span),
                None => line.span,
            });
        } else {
            body_lines.push(trimmed.to_string());
        }
    }

    if let Some(tag) = current_tag.take() {
        tags.push(tag);
    }

    let body = body_lines.join("\n").trim().to_string();
    let summary = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .unwrap_or_default();

    DocComment {
        span,
        summary,
        body,
        tags,
    }
}

fn parse_tag_line(line: &DocLine) -> Option<DocTag> {
    let leading = trim_start_offset(&line.text);
    if line.text[leading..].chars().next()? != '@' {
        return None;
    }

    let tag_name_start = leading + 1;
    let tag_name_end = token_end_offset(&line.text, tag_name_start);
    let remainder_start = skip_whitespace_offset(&line.text, tag_name_end);
    let tag_name = &line.text[tag_name_start..tag_name_end];
    let remainder = &line.text[remainder_start..];
    let kind = match tag_name {
        "module" => DocTagKind::Module,
        "typeparam" => DocTagKind::TypeParam,
        "param" => DocTagKind::Param,
        "returns" => DocTagKind::Returns,
        "throws" => DocTagKind::Throws,
        "deprecated" => DocTagKind::Deprecated,
        "requires" => DocTagKind::Requires,
        "since" => DocTagKind::Since,
        "see" => DocTagKind::See,
        "link" => DocTagKind::Link,
        "note" => DocTagKind::Note,
        "example" => DocTagKind::Example,
        other => DocTagKind::Unknown(other.to_string()),
    };

    let tag_span = span_from_offsets(line.span, leading, line.text.len());
    let kind_span = span_from_offsets(line.span, tag_name_start, tag_name_end);

    Some(match kind {
        DocTagKind::TypeParam | DocTagKind::Param => {
            let name_start = remainder_start;
            let name_end = token_end_offset(&line.text, name_start);
            let body_start = skip_whitespace_offset(&line.text, name_end);
            let name = &line.text[name_start..name_end];
            let body = &line.text[body_start..];
            DocTag {
                kind,
                span: tag_span,
                kind_span,
                name: (!name.is_empty()).then(|| name.to_string()),
                name_span: (!name.is_empty()).then(|| span_from_offsets(line.span, name_start, name_end)),
                body: body.trim().to_string(),
                body_span: (!body.trim().is_empty())
                    .then(|| span_from_offsets(line.span, body_start, line.text.len())),
                link: None,
            }
        }
        DocTagKind::See => DocTag {
            kind,
            span: tag_span,
            kind_span,
            name: None,
            name_span: None,
            body: remainder.trim().to_string(),
            body_span: (!remainder.trim().is_empty())
                .then(|| span_from_offsets(line.span, remainder_start, line.text.len())),
            link: parse_link(line, remainder_start, false),
        },
        DocTagKind::Link => DocTag {
            kind,
            span: tag_span,
            kind_span,
            name: None,
            name_span: None,
            body: remainder.trim().to_string(),
            body_span: (!remainder.trim().is_empty())
                .then(|| span_from_offsets(line.span, remainder_start, line.text.len())),
            link: parse_link(line, remainder_start, true),
        },
        _ => DocTag {
            kind,
            span: tag_span,
            kind_span,
            name: None,
            name_span: None,
            body: remainder.trim().to_string(),
            body_span: (!remainder.trim().is_empty())
                .then(|| span_from_offsets(line.span, remainder_start, line.text.len())),
            link: None,
        },
    })
}

fn parse_link(line: &DocLine, start_offset: usize, allow_label: bool) -> Option<DocLink> {
    let trimmed_start = skip_whitespace_offset(&line.text, start_offset);
    let trimmed = &line.text[trimmed_start..];
    if trimmed.is_empty() {
        return None;
    }
    let target_end = token_end_offset(&line.text, trimmed_start);
    let target = &line.text[trimmed_start..target_end];
    let label_start = skip_whitespace_offset(&line.text, target_end);
    let label = allow_label.then(|| line.text[label_start..].trim().to_string());
    let label = label.filter(|value| !value.is_empty());
    let label_span = label
        .as_ref()
        .map(|_| span_from_offsets(line.span, label_start, line.text.len()));
    Some(DocLink {
        target: target.to_string(),
        target_span: span_from_offsets(line.span, trimmed_start, target_end),
        label,
        label_span,
    })
}

fn trim_start_offset(text: &str) -> usize {
    text.char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

fn skip_whitespace_offset(text: &str, start: usize) -> usize {
    let tail = &text[start.min(text.len())..];
    start
        + tail
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(tail.len())
}

fn token_end_offset(text: &str, start: usize) -> usize {
    let tail = &text[start.min(text.len())..];
    start
        + tail
            .char_indices()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(tail.len())
}

fn span_from_offsets(base: Span, start: usize, end: usize) -> Span {
    Span::new(base.start + start, (base.start + end).min(base.end))
}

#[derive(Default)]
struct DocCollector {
    entries: Vec<DocEntry>,
}

impl DocCollector {
    fn collect_items(&mut self, items: &[Item], module_path: &[String]) {
        for item in items {
            self.collect_item(item, module_path);
        }
    }

    fn collect_item(&mut self, item: &Item, module_path: &[String]) {
        match item {
            Item::Module(module, span) => {
                let path = join_path(module_path, &module.name);
                self.attach_comment(
                    DocTargetKind::Module,
                    path.clone(),
                    *span,
                    module.doc_comment.as_ref(),
                );
                self.collect_items(&module.items, &append_path(module_path, &module.name));
            }
            Item::Function(function, span) => {
                let path = join_path(module_path, &function.name);
                self.attach_comment(
                    DocTargetKind::Function,
                    path.clone(),
                    *span,
                    function.doc_comment.as_ref(),
                );
                self.collect_type_params(&path, function.type_params.as_deref());
            }
            Item::AnnotationDef(annotation_def, span) => {
                let path = join_annotation_path(module_path, &annotation_def.name);
                self.attach_comment(
                    DocTargetKind::Annotation,
                    path.clone(),
                    *span,
                    annotation_def.doc_comment.as_ref(),
                );
            }
            Item::ForeignFunction(function, span) => {
                let path = join_path(module_path, &function.name);
                self.attach_comment(
                    DocTargetKind::ForeignFunction,
                    path.clone(),
                    *span,
                    function.doc_comment.as_ref(),
                );
                self.collect_type_params(&path, function.type_params.as_deref());
            }
            Item::BuiltinFunctionDecl(function, span) => {
                let path = join_path(module_path, &function.name);
                self.attach_comment(
                    DocTargetKind::BuiltinFunction,
                    path.clone(),
                    *span,
                    function.doc_comment.as_ref(),
                );
                self.collect_type_params(&path, function.type_params.as_deref());
            }
            Item::BuiltinTypeDecl(ty, span) => {
                let path = join_path(module_path, &ty.name);
                self.attach_comment(
                    DocTargetKind::BuiltinType,
                    path.clone(),
                    *span,
                    ty.doc_comment.as_ref(),
                );
                self.collect_type_params(&path, ty.type_params.as_deref());
            }
            Item::TypeAlias(alias, span) => {
                let path = join_path(module_path, &alias.name);
                self.attach_comment(
                    DocTargetKind::TypeAlias,
                    path.clone(),
                    *span,
                    alias.doc_comment.as_ref(),
                );
                self.collect_type_params(&path, alias.type_params.as_deref());
            }
            Item::StructType(struct_def, span) => {
                let path = join_path(module_path, &struct_def.name);
                self.collect_struct(&path, *span, struct_def.doc_comment.as_ref(), struct_def);
            }
            Item::Enum(enum_def, span) => {
                let path = join_path(module_path, &enum_def.name);
                self.collect_enum(&path, *span, enum_def.doc_comment.as_ref(), enum_def);
            }
            Item::Interface(interface_def, span) => {
                let path = join_path(module_path, &interface_def.name);
                self.collect_interface(&path, *span, interface_def.doc_comment.as_ref(), interface_def);
            }
            Item::Trait(trait_def, span) => {
                let path = join_path(module_path, &trait_def.name);
                self.collect_trait(&path, *span, trait_def.doc_comment.as_ref(), trait_def);
            }
            Item::Export(export, span) => match &export.item {
                ExportItem::Function(function) => {
                    let path = join_path(module_path, &function.name);
                    self.attach_comment(
                        DocTargetKind::Function,
                        path.clone(),
                        *span,
                        function.doc_comment.as_ref(),
                    );
                    self.collect_type_params(&path, function.type_params.as_deref());
                }
                ExportItem::ForeignFunction(function) => {
                    let path = join_path(module_path, &function.name);
                    self.attach_comment(
                        DocTargetKind::ForeignFunction,
                        path.clone(),
                        *span,
                        function.doc_comment.as_ref(),
                    );
                    self.collect_type_params(&path, function.type_params.as_deref());
                }
                ExportItem::TypeAlias(alias) => {
                    let path = join_path(module_path, &alias.name);
                    self.attach_comment(
                        DocTargetKind::TypeAlias,
                        path.clone(),
                        *span,
                        alias.doc_comment.as_ref(),
                    );
                    self.collect_type_params(&path, alias.type_params.as_deref());
                }
                ExportItem::Struct(struct_def) => {
                    let path = join_path(module_path, &struct_def.name);
                    self.collect_struct(&path, *span, struct_def.doc_comment.as_ref(), struct_def);
                }
                ExportItem::Enum(enum_def) => {
                    let path = join_path(module_path, &enum_def.name);
                    self.collect_enum(&path, *span, enum_def.doc_comment.as_ref(), enum_def);
                }
                ExportItem::Interface(interface_def) => {
                    let path = join_path(module_path, &interface_def.name);
                    self.collect_interface(
                        &path,
                        *span,
                        interface_def.doc_comment.as_ref(),
                        interface_def,
                    );
                }
                ExportItem::Trait(trait_def) => {
                    let path = join_path(module_path, &trait_def.name);
                    self.collect_trait(&path, *span, trait_def.doc_comment.as_ref(), trait_def);
                }
                ExportItem::Named(_) => {}
            },
            _ => {}
        }
    }

    fn collect_struct(
        &mut self,
        path: &str,
        span: Span,
        doc_comment: Option<&DocComment>,
        struct_def: &crate::ast::StructTypeDef,
    ) {
        self.attach_comment(DocTargetKind::Struct, path.to_string(), span, doc_comment);
        self.collect_type_params(path, struct_def.type_params.as_deref());
        for field in &struct_def.fields {
            self.attach_comment(
                DocTargetKind::StructField,
                join_child_path(path, &field.name),
                field.span,
                field.doc_comment.as_ref(),
            );
        }
    }

    fn collect_enum(
        &mut self,
        path: &str,
        span: Span,
        doc_comment: Option<&DocComment>,
        enum_def: &crate::ast::EnumDef,
    ) {
        self.attach_comment(DocTargetKind::Enum, path.to_string(), span, doc_comment);
        self.collect_type_params(path, enum_def.type_params.as_deref());
        for member in &enum_def.members {
            self.attach_comment(
                DocTargetKind::EnumVariant,
                join_child_path(path, &member.name),
                member.span,
                member.doc_comment.as_ref(),
            );
        }
    }

    fn collect_interface(
        &mut self,
        path: &str,
        span: Span,
        doc_comment: Option<&DocComment>,
        interface_def: &crate::ast::InterfaceDef,
    ) {
        self.attach_comment(DocTargetKind::Interface, path.to_string(), span, doc_comment);
        self.collect_type_params(path, interface_def.type_params.as_deref());
        for member in &interface_def.members {
            let (kind, name) = match member {
                crate::ast::InterfaceMember::Property { name, .. } => {
                    (DocTargetKind::InterfaceProperty, name.as_str())
                }
                crate::ast::InterfaceMember::Method { name, .. } => {
                    (DocTargetKind::InterfaceMethod, name.as_str())
                }
                crate::ast::InterfaceMember::IndexSignature { param_type, .. } => {
                    (DocTargetKind::InterfaceIndexSignature, param_type.as_str())
                }
            };
            let child_name = if matches!(kind, DocTargetKind::InterfaceIndexSignature) {
                format!("[{}]", name)
            } else {
                name.to_string()
            };
            self.attach_comment(
                kind,
                join_child_path(path, &child_name),
                member.span(),
                member.doc_comment(),
            );
        }
    }

    fn collect_trait(
        &mut self,
        path: &str,
        span: Span,
        doc_comment: Option<&DocComment>,
        trait_def: &crate::ast::TraitDef,
    ) {
        self.attach_comment(DocTargetKind::Trait, path.to_string(), span, doc_comment);
        self.collect_type_params(path, trait_def.type_params.as_deref());
        for member in &trait_def.members {
            let (kind, child_name, child_span) = match member {
                TraitMember::Required(crate::ast::InterfaceMember::Property { name, span, .. })
                | TraitMember::Required(crate::ast::InterfaceMember::Method { name, span, .. }) => {
                    (DocTargetKind::TraitMethod, name.clone(), *span)
                }
                TraitMember::Required(crate::ast::InterfaceMember::IndexSignature {
                    param_type, span, ..
                }) => (DocTargetKind::TraitMethod, format!("[{}]", param_type), *span),
                TraitMember::Default(method) => {
                    (DocTargetKind::TraitMethod, method.name.clone(), method.span)
                }
                TraitMember::AssociatedType { name, span, .. } => {
                    (DocTargetKind::TraitAssociatedType, name.clone(), *span)
                }
            };
            self.attach_comment(
                kind,
                join_child_path(path, &child_name),
                child_span,
                member.doc_comment(),
            );
        }
    }

    fn collect_type_params(&mut self, parent_path: &str, type_params: Option<&[TypeParam]>) {
        for type_param in type_params.unwrap_or(&[]) {
            self.attach_comment(
                DocTargetKind::TypeParam,
                join_type_param_path(parent_path, &type_param.name),
                type_param.span,
                type_param.doc_comment.as_ref(),
            );
        }
    }

    fn attach_comment(
        &mut self,
        kind: DocTargetKind,
        path: String,
        span: Span,
        doc_comment: Option<&DocComment>,
    ) {
        let Some(comment) = doc_comment.cloned() else {
            return;
        };
        self.entries.push(DocEntry {
            target: DocTarget { kind, path, span },
            comment,
        });
    }
}

fn append_path(module_path: &[String], name: &str) -> Vec<String> {
    let mut next = module_path.to_vec();
    next.push(name.to_string());
    next
}

fn join_path(prefix: &[String], name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", prefix.join("::"), name)
    }
}

fn join_annotation_path(prefix: &[String], name: &str) -> String {
    join_path(prefix, &format!("@{name}"))
}

fn join_child_path(parent: &str, name: &str) -> String {
    format!("{}::{}", parent, name)
}

fn join_type_param_path(parent: &str, name: &str) -> String {
    format!("{}::<{}>", parent, name)
}

#[cfg(test)]
mod tests {
    use crate::ast::DocTargetKind;
    use crate::parser::parse_program;

    #[test]
    fn attaches_docs_to_top_level_items() {
        let program = parse_program("/// Adds\nfn add(x: number) -> number { x }\n")
            .expect("program should parse");
        let doc = program
            .docs
            .comment_for_path("add")
            .expect("doc for function");
        assert_eq!(doc.summary, "Adds");
    }

    #[test]
    fn attaches_docs_to_struct_members() {
        let source = "type Point {\n    /// X coordinate\n    x: number,\n}\n";
        let program = parse_program(source).expect("program should parse");
        let doc = program
            .docs
            .comment_for_path("Point::x")
            .expect("doc for field");
        assert_eq!(doc.summary, "X coordinate");
    }

    #[test]
    fn attaches_docs_to_type_params() {
        let source = "fn identity<\n    /// Input type\n    T\n>(value: T) -> T { value }\n";
        let program = parse_program(source).expect("program should parse");
        let entry = program
            .docs
            .entry_for_path("identity::<T>")
            .expect("doc for type param");
        assert_eq!(entry.target.kind, DocTargetKind::TypeParam);
        assert_eq!(entry.comment.summary, "Input type");
    }

    #[test]
    fn parses_structured_tags() {
        let source = "/// Summary\n/// @param x value\nfn add(x: number) -> number { x }\n";
        let program = parse_program(source).expect("program should parse");
        let doc = program
            .docs
            .comment_for_path("add")
            .expect("doc for function");
        assert_eq!(doc.param_doc("x"), Some("value"));
    }

    #[test]
    fn attaches_docs_to_annotation_defs() {
        let source = "/// Configures warmup handling.\n/// @param period Number of lookback bars.\nannotation warmup(period) { metadata() { return { warmup: period } } }\n";
        let program = parse_program(source).expect("program should parse");
        let entry = program
            .docs
            .entry_for_path("@warmup")
            .expect("doc for annotation");
        assert_eq!(entry.target.kind, DocTargetKind::Annotation);
        assert_eq!(
            entry.comment.param_doc("period"),
            Some("Number of lookback bars.")
        );
    }

    #[test]
    fn block_doc_comments_do_not_create_docs() {
        let source = "/** Old style */\nfn add(x: number) -> number { x }\n";
        let program = parse_program(source).expect("program should parse");
        assert!(program.docs.comment_for_path("add").is_none());
    }
}
