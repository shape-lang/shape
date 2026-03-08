use crate::util::span_to_range;
use shape_ast::ast::{ExportItem, Item, Span, TraitMember, TypeAnnotation, TypeParam};
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Position, Range, TextEdit, Uri,
    WorkspaceEdit,
};

pub fn generate_doc_comment_action(
    text: &str,
    uri: &Uri,
    range: Range,
) -> Option<CodeActionOrCommand> {
    let program = parse_program(text).ok()?;
    let target = find_doc_target(&program.items, text, range.start.line)?;
    if target.has_doc {
        return None;
    }

    let indent = line_indent(text, target.span);
    let new_text = render_doc_stub(&target, &indent);
    let insert_line = span_to_range(text, &target.span).start.line;

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: "Generate doc comment".to_string(),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(std::collections::HashMap::from([(
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: insert_line,
                            character: 0,
                        },
                        end: Position {
                            line: insert_line,
                            character: 0,
                        },
                    },
                    new_text,
                }],
            )])),
            ..Default::default()
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }))
}

#[derive(Clone)]
struct DocTemplateTarget {
    span: Span,
    has_doc: bool,
    type_params: Vec<String>,
    params: Vec<String>,
    has_returns: bool,
}

fn find_doc_target(items: &[Item], text: &str, line: u32) -> Option<DocTemplateTarget> {
    for item in items {
        match item {
            Item::Module(module, _) => {
                if let Some(target) = find_doc_target(&module.items, text, line) {
                    return Some(target);
                }
            }
            Item::AnnotationDef(annotation_def, span) if starts_on_line(text, *span, line) => {
                return Some(callable_target(
                    *span,
                    annotation_def.doc_comment.is_some(),
                    None,
                    annotation_def
                        .params
                        .iter()
                        .flat_map(|param| param.get_identifiers()),
                    false,
                ));
            }
            Item::Function(function, span) if starts_on_line(text, *span, line) => {
                return Some(callable_target(
                    *span,
                    function.doc_comment.is_some(),
                    function.type_params.as_deref(),
                    function.params.iter().flat_map(|param| param.get_identifiers()),
                    function
                        .return_type
                        .as_ref()
                        .is_some_and(|ty| !matches!(ty, TypeAnnotation::Void)),
                ));
            }
            Item::ForeignFunction(function, span) if starts_on_line(text, *span, line) => {
                return Some(callable_target(
                    *span,
                    function.doc_comment.is_some(),
                    function.type_params.as_deref(),
                    function.params.iter().flat_map(|param| param.get_identifiers()),
                    function
                        .return_type
                        .as_ref()
                        .is_some_and(|ty| !matches!(ty, TypeAnnotation::Void)),
                ));
            }
            Item::BuiltinFunctionDecl(function, span) if starts_on_line(text, *span, line) => {
                return Some(callable_target(
                    *span,
                    function.doc_comment.is_some(),
                    function.type_params.as_deref(),
                    function.params.iter().flat_map(|param| param.get_identifiers()),
                    !matches!(function.return_type, TypeAnnotation::Void),
                ));
            }
            Item::BuiltinTypeDecl(ty, span) if starts_on_line(text, *span, line) => {
                return Some(type_target(*span, ty.doc_comment.is_some(), ty.type_params.as_deref()));
            }
            Item::TypeAlias(alias, span) if starts_on_line(text, *span, line) => {
                return Some(type_target(
                    *span,
                    alias.doc_comment.is_some(),
                    alias.type_params.as_deref(),
                ));
            }
            Item::StructType(struct_def, span) => {
                if starts_on_line(text, *span, line) {
                    return Some(type_target(
                        *span,
                        struct_def.doc_comment.is_some(),
                        struct_def.type_params.as_deref(),
                    ));
                }
                for field in &struct_def.fields {
                    if starts_on_line(text, field.span, line) {
                        return Some(leaf_target(field.span, field.doc_comment.is_some()));
                    }
                }
            }
            Item::Enum(enum_def, span) => {
                if starts_on_line(text, *span, line) {
                    return Some(type_target(
                        *span,
                        enum_def.doc_comment.is_some(),
                        enum_def.type_params.as_deref(),
                    ));
                }
                for member in &enum_def.members {
                    if starts_on_line(text, member.span, line) {
                        return Some(leaf_target(member.span, member.doc_comment.is_some()));
                    }
                }
            }
            Item::Interface(interface, span) => {
                if starts_on_line(text, *span, line) {
                    return Some(type_target(
                        *span,
                        interface.doc_comment.is_some(),
                        interface.type_params.as_deref(),
                    ));
                }
                for member in &interface.members {
                    let span = member.span();
                    if starts_on_line(text, span, line) {
                        return Some(match member {
                            shape_ast::ast::InterfaceMember::Method {
                                params,
                                return_type,
                                doc_comment,
                                ..
                            } => callable_target(
                                span,
                                doc_comment.is_some(),
                                None,
                                params.iter().filter_map(|param| param.name.clone()),
                                !matches!(return_type, TypeAnnotation::Void),
                            ),
                            shape_ast::ast::InterfaceMember::Property { doc_comment, .. }
                            | shape_ast::ast::InterfaceMember::IndexSignature {
                                doc_comment, ..
                            } => leaf_target(span, doc_comment.is_some()),
                        });
                    }
                }
            }
            Item::Trait(trait_def, span) => {
                if starts_on_line(text, *span, line) {
                    return Some(type_target(
                        *span,
                        trait_def.doc_comment.is_some(),
                        trait_def.type_params.as_deref(),
                    ));
                }
                for member in &trait_def.members {
                    let span = member.span();
                    if starts_on_line(text, span, line) {
                        return Some(match member {
                            TraitMember::Default(method) => callable_target(
                                span,
                                method.doc_comment.is_some(),
                                None,
                                method.params.iter().flat_map(|param| param.get_identifiers()),
                                method
                                    .return_type
                                    .as_ref()
                                    .is_some_and(|ty| !matches!(ty, TypeAnnotation::Void)),
                            ),
                            TraitMember::Required(shape_ast::ast::InterfaceMember::Method {
                                params,
                                return_type,
                                doc_comment,
                                ..
                            }) => callable_target(
                                span,
                                doc_comment.is_some(),
                                None,
                                params.iter().filter_map(|param| param.name.clone()),
                                !matches!(return_type, TypeAnnotation::Void),
                            ),
                            TraitMember::Required(shape_ast::ast::InterfaceMember::Property {
                                doc_comment, ..
                            })
                            | TraitMember::Required(
                                shape_ast::ast::InterfaceMember::IndexSignature {
                                    doc_comment, ..
                                },
                            )
                            | TraitMember::AssociatedType { doc_comment, .. } => {
                                leaf_target(span, doc_comment.is_some())
                            }
                        });
                    }
                }
            }
            Item::Export(export, span) if starts_on_line(text, *span, line) => {
                return Some(match &export.item {
                    ExportItem::Function(function) => callable_target(
                        *span,
                        function.doc_comment.is_some(),
                        function.type_params.as_deref(),
                        function.params.iter().flat_map(|param| param.get_identifiers()),
                        function
                            .return_type
                            .as_ref()
                            .is_some_and(|ty| !matches!(ty, TypeAnnotation::Void)),
                    ),
                    ExportItem::ForeignFunction(function) => callable_target(
                        *span,
                        function.doc_comment.is_some(),
                        function.type_params.as_deref(),
                        function.params.iter().flat_map(|param| param.get_identifiers()),
                        function
                            .return_type
                            .as_ref()
                            .is_some_and(|ty| !matches!(ty, TypeAnnotation::Void)),
                    ),
                    ExportItem::TypeAlias(alias) => {
                        type_target(*span, alias.doc_comment.is_some(), alias.type_params.as_deref())
                    }
                    ExportItem::Struct(struct_def) => type_target(
                        *span,
                        struct_def.doc_comment.is_some(),
                        struct_def.type_params.as_deref(),
                    ),
                    ExportItem::Enum(enum_def) => {
                        type_target(*span, enum_def.doc_comment.is_some(), enum_def.type_params.as_deref())
                    }
                    ExportItem::Interface(interface) => type_target(
                        *span,
                        interface.doc_comment.is_some(),
                        interface.type_params.as_deref(),
                    ),
                    ExportItem::Trait(trait_def) => {
                        type_target(*span, trait_def.doc_comment.is_some(), trait_def.type_params.as_deref())
                    }
                    ExportItem::Named(_) => continue,
                });
            }
            _ => {}
        }
    }

    None
}

fn callable_target(
    span: Span,
    has_doc: bool,
    type_params: Option<&[TypeParam]>,
    params: impl Iterator<Item = String>,
    has_returns: bool,
) -> DocTemplateTarget {
    let mut param_names: Vec<_> = params.collect();
    param_names.sort();
    param_names.dedup();

    DocTemplateTarget {
        span,
        has_doc,
        type_params: type_param_names(type_params),
        params: param_names,
        has_returns,
    }
}

fn type_target(span: Span, has_doc: bool, type_params: Option<&[TypeParam]>) -> DocTemplateTarget {
    DocTemplateTarget {
        span,
        has_doc,
        type_params: type_param_names(type_params),
        params: Vec::new(),
        has_returns: false,
    }
}

fn leaf_target(span: Span, has_doc: bool) -> DocTemplateTarget {
    DocTemplateTarget {
        span,
        has_doc,
        type_params: Vec::new(),
        params: Vec::new(),
        has_returns: false,
    }
}

fn type_param_names(type_params: Option<&[TypeParam]>) -> Vec<String> {
    type_params
        .unwrap_or(&[])
        .iter()
        .map(|param| param.name.clone())
        .collect()
}

fn render_doc_stub(target: &DocTemplateTarget, indent: &str) -> String {
    let mut lines = vec![format!("{indent}/// Summary.")];
    for type_param in &target.type_params {
        lines.push(format!("{indent}/// @typeparam {type_param} Describe `{type_param}`."));
    }
    for param in &target.params {
        lines.push(format!("{indent}/// @param {param} Describe `{param}`."));
    }
    if target.has_returns {
        lines.push(format!("{indent}/// @returns Describe the return value."));
    }
    format!("{}\n", lines.join("\n"))
}

fn starts_on_line(text: &str, span: Span, line: u32) -> bool {
    span_to_range(text, &span).start.line == line
}

fn line_indent(text: &str, span: Span) -> String {
    let start_line = span_to_range(text, &span).start.line as usize;
    text.lines()
        .nth(start_line)
        .map(|line| {
            line.chars()
                .take_while(|ch| ch.is_whitespace())
                .collect::<String>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp_server::ls_types::Uri;

    #[test]
    fn generates_function_doc_stub() {
        let text = "fn add(value: number) -> number { value }\n";
        let uri = Uri::from_file_path("/tmp/test.shape").expect("valid file uri");
        let action = generate_doc_comment_action(
            text,
            &uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
        )
        .expect("doc action should be available");

        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!("expected code action");
        };
        let edits = action
            .edit
            .and_then(|edit| edit.changes)
            .expect("workspace edits");
        let new_text = &edits
            .values()
            .next()
            .expect("doc edits")[0]
            .new_text;
        assert!(new_text.contains("/// Summary."));
        assert!(new_text.contains("/// @param value"));
        assert!(new_text.contains("/// @returns"));
    }

    #[test]
    fn does_not_offer_action_when_doc_exists() {
        let text = "/// Summary.\nfn add(value: number) -> number { value }\n";
        let uri = Uri::from_file_path("/tmp/test.shape").expect("valid file uri");
        let action = generate_doc_comment_action(
            text,
            &uri,
            Range {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 0,
                },
            },
        );
        assert!(action.is_none());
    }

    #[test]
    fn generates_annotation_doc_stub() {
        let text = "annotation warmup(period) {\n    metadata() { return { warmup: period } }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").expect("valid file uri");
        let action = generate_doc_comment_action(
            text,
            &uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
        )
        .expect("doc action should be available");

        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!("expected code action");
        };
        let edits = action
            .edit
            .and_then(|edit| edit.changes)
            .expect("workspace edits");
        let new_text = &edits
            .values()
            .next()
            .expect("doc edits")[0]
            .new_text;
        assert!(new_text.contains("/// Summary."));
        assert!(new_text.contains("/// @param period"));
        assert!(!new_text.contains("/// @returns"));
    }
}
