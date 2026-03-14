use serde::{Deserialize, Serialize};
use shape_ast::ast::{
    DocComment, ExportItem, FunctionDef, InterfaceMember, Item, Program, Span, TraitMember,
    TypeAnnotation,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DocItemKind {
    Function,
    Type,
    Interface,
    Enum,
    Trait,
    Field,
    Variant,
    Method,
    AssociatedType,
    Constant,
    Module,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocParam {
    pub name: String,
    pub type_name: Option<String>,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageDocs {
    pub readme: Option<String>,
    pub modules: HashMap<String, Vec<DocItem>>,
}

pub fn extract_docs_from_ast(_source: &str, ast: &Program) -> Vec<DocItem> {
    let mut docs = Vec::new();
    collect_items(&ast.items, ast, &[], &mut docs);
    docs
}

fn collect_items(
    items: &[Item],
    program: &Program,
    module_path: &[String],
    docs: &mut Vec<DocItem>,
) {
    for item in items {
        match item {
            Item::Module(module, span) => {
                let path = join_path(module_path, &module.name);
                if let Some(comment) = program.docs.comment_for_span(*span) {
                    docs.push(DocItem {
                        kind: DocItemKind::Module,
                        name: path.clone(),
                        doc: doc_text(comment),
                        signature: None,
                        type_params: Vec::new(),
                        params: Vec::new(),
                        return_type: None,
                        children: Vec::new(),
                    });
                }

                let mut next_path = module_path.to_vec();
                next_path.push(module.name.clone());
                collect_items(&module.items, program, &next_path, docs);
            }
            Item::Function(function, span) => {
                docs.push(extract_function_doc(
                    program,
                    join_path(module_path, &function.name),
                    function,
                    *span,
                ));
            }
            Item::ForeignFunction(function, span) => {
                docs.push(extract_function_doc(
                    program,
                    join_path(module_path, &function.name),
                    &FunctionDef {
                        name: function.name.clone(),
                        name_span: function.name_span,
                        declaring_module_path: None,
                        doc_comment: function.doc_comment.clone(),
                        type_params: function.type_params.clone(),
                        params: function.params.clone(),
                        return_type: function.return_type.clone(),
                        where_clause: None,
                        body: Vec::new(),
                        annotations: function.annotations.clone(),
                        is_async: function.is_async,
                        is_comptime: false,
                    },
                    *span,
                ));
            }
            Item::StructType(struct_def, span) => {
                docs.push(extract_struct_doc(
                    program,
                    join_path(module_path, &struct_def.name),
                    struct_def,
                    *span,
                ));
            }
            Item::Enum(enum_def, span) => {
                docs.push(extract_enum_doc(
                    program,
                    join_path(module_path, &enum_def.name),
                    enum_def,
                    *span,
                ));
            }
            Item::Trait(trait_def, span) => {
                docs.push(extract_trait_doc(
                    program,
                    join_path(module_path, &trait_def.name),
                    trait_def,
                    *span,
                ));
            }
            Item::Interface(interface_def, span) => {
                docs.push(extract_interface_doc(
                    program,
                    join_path(module_path, &interface_def.name),
                    interface_def,
                    *span,
                ));
            }
            Item::TypeAlias(alias, span) => {
                let path = join_path(module_path, &alias.name);
                docs.push(DocItem {
                    kind: DocItemKind::Type,
                    name: path.clone(),
                    doc: doc_text_from_span(program, *span),
                    signature: Some(format!(
                        "type {} = {}",
                        alias.name,
                        format_type_annotation(&alias.type_annotation)
                    )),
                    type_params: format_type_params(&alias.type_params),
                    params: Vec::new(),
                    return_type: Some(format_type_annotation(&alias.type_annotation)),
                    children: Vec::new(),
                });
            }
            Item::BuiltinFunctionDecl(func, span) => {
                docs.push(DocItem {
                    kind: DocItemKind::Function,
                    name: join_path(module_path, &func.name),
                    doc: doc_text_from_span(program, *span),
                    signature: Some(format_builtin_signature(func)),
                    type_params: format_type_params(&func.type_params),
                    params: func
                        .params
                        .iter()
                        .map(|param| DocParam {
                            name: param.simple_name().unwrap_or("_").to_string(),
                            type_name: param.type_annotation.as_ref().map(format_type_annotation),
                            description: program
                                .docs
                                .comment_for_span(*span)
                                .and_then(|doc| doc.param_doc(param.simple_name().unwrap_or("_")))
                                .map(str::to_string),
                            default_value: None,
                        })
                        .collect(),
                    return_type: Some(format_type_annotation(&func.return_type)),
                    children: Vec::new(),
                });
            }
            Item::BuiltinTypeDecl(ty, span) => {
                docs.push(DocItem {
                    kind: DocItemKind::Type,
                    name: join_path(module_path, &ty.name),
                    doc: doc_text_from_span(program, *span),
                    signature: Some(format!("builtin type {}", ty.name)),
                    type_params: format_type_params(&ty.type_params),
                    params: Vec::new(),
                    return_type: None,
                    children: Vec::new(),
                });
            }
            Item::Export(export, span) => match &export.item {
                ExportItem::Function(function) => {
                    docs.push(extract_function_doc(
                        program,
                        join_path(module_path, &function.name),
                        function,
                        *span,
                    ));
                }
                ExportItem::BuiltinFunction(function) => {
                    docs.push(DocItem {
                        kind: DocItemKind::Function,
                        name: join_path(module_path, &function.name),
                        doc: doc_text_from_span(program, *span),
                        signature: Some(format_builtin_signature(function)),
                        type_params: format_type_params(&function.type_params),
                        params: function
                            .params
                            .iter()
                            .map(|param| DocParam {
                                name: param.simple_name().unwrap_or("_").to_string(),
                                type_name: param
                                    .type_annotation
                                    .as_ref()
                                    .map(format_type_annotation),
                                description: program
                                    .docs
                                    .comment_for_span(*span)
                                    .and_then(|doc| {
                                        doc.param_doc(param.simple_name().unwrap_or("_"))
                                    })
                                    .map(str::to_string),
                                default_value: None,
                            })
                            .collect(),
                        return_type: Some(format_type_annotation(&function.return_type)),
                        children: Vec::new(),
                    });
                }
                ExportItem::ForeignFunction(function) => {
                    docs.push(DocItem {
                        kind: DocItemKind::Function,
                        name: join_path(module_path, &function.name),
                        doc: doc_text_from_span(program, *span),
                        signature: Some(format_foreign_signature(function)),
                        type_params: format_type_params(&function.type_params),
                        params: function
                            .params
                            .iter()
                            .map(|param| DocParam {
                                name: param.simple_name().unwrap_or("_").to_string(),
                                type_name: param
                                    .type_annotation
                                    .as_ref()
                                    .map(format_type_annotation),
                                description: program
                                    .docs
                                    .comment_for_span(*span)
                                    .and_then(|doc| {
                                        doc.param_doc(param.simple_name().unwrap_or("_"))
                                    })
                                    .map(str::to_string),
                                default_value: None,
                            })
                            .collect(),
                        return_type: function.return_type.as_ref().map(format_type_annotation),
                        children: Vec::new(),
                    });
                }
                ExportItem::Struct(struct_def) => {
                    docs.push(extract_struct_doc(
                        program,
                        join_path(module_path, &struct_def.name),
                        struct_def,
                        *span,
                    ));
                }
                ExportItem::Enum(enum_def) => {
                    docs.push(extract_enum_doc(
                        program,
                        join_path(module_path, &enum_def.name),
                        enum_def,
                        *span,
                    ));
                }
                ExportItem::Trait(trait_def) => {
                    docs.push(extract_trait_doc(
                        program,
                        join_path(module_path, &trait_def.name),
                        trait_def,
                        *span,
                    ));
                }
                ExportItem::Interface(interface_def) => {
                    docs.push(extract_interface_doc(
                        program,
                        join_path(module_path, &interface_def.name),
                        interface_def,
                        *span,
                    ));
                }
                ExportItem::TypeAlias(alias) => {
                    docs.push(DocItem {
                        kind: DocItemKind::Type,
                        name: join_path(module_path, &alias.name),
                        doc: doc_text_from_span(program, *span),
                        signature: Some(format!(
                            "type {} = {}",
                            alias.name,
                            format_type_annotation(&alias.type_annotation)
                        )),
                        type_params: format_type_params(&alias.type_params),
                        params: Vec::new(),
                        return_type: Some(format_type_annotation(&alias.type_annotation)),
                        children: Vec::new(),
                    });
                }
                ExportItem::BuiltinType(ty) => {
                    docs.push(DocItem {
                        kind: DocItemKind::Type,
                        name: join_path(module_path, &ty.name),
                        doc: doc_text_from_span(program, *span),
                        signature: Some(format!("builtin type {}", ty.name)),
                        type_params: format_type_params(&ty.type_params),
                        params: Vec::new(),
                        return_type: None,
                        children: Vec::new(),
                    });
                }
                ExportItem::Annotation(_) => {}
                ExportItem::Named(_) => {}
            },
            _ => {}
        }
    }
}

fn extract_function_doc(
    program: &Program,
    path: String,
    func: &FunctionDef,
    span: Span,
) -> DocItem {
    let doc = program.docs.comment_for_span(span);
    let params = func
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("_").to_string();
            DocParam {
                description: doc.and_then(|d| d.param_doc(&name)).map(str::to_string),
                default_value: None,
                name,
                type_name: param.type_annotation.as_ref().map(format_type_annotation),
            }
        })
        .collect();

    DocItem {
        kind: DocItemKind::Function,
        name: path,
        doc: doc.map(doc_text).unwrap_or_default(),
        signature: Some(format_function_signature(func)),
        type_params: format_type_params(&func.type_params),
        params,
        return_type: func.return_type.as_ref().map(format_type_annotation),
        children: Vec::new(),
    }
}

fn extract_struct_doc(
    program: &Program,
    path: String,
    st: &shape_ast::ast::StructTypeDef,
    span: Span,
) -> DocItem {
    let children = st
        .fields
        .iter()
        .map(|field| DocItem {
            kind: DocItemKind::Field,
            name: join_child_path(&path, &field.name),
            doc: doc_text_from_span(program, field.span),
            signature: Some(format!(
                "{}: {}",
                field.name,
                format_type_annotation(&field.type_annotation)
            )),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(format_type_annotation(&field.type_annotation)),
            children: Vec::new(),
        })
        .collect();

    DocItem {
        kind: DocItemKind::Type,
        name: path,
        doc: doc_text_from_span(program, span),
        signature: None,
        type_params: format_type_params(&st.type_params),
        params: Vec::new(),
        return_type: None,
        children,
    }
}

fn extract_enum_doc(
    program: &Program,
    path: String,
    en: &shape_ast::ast::EnumDef,
    span: Span,
) -> DocItem {
    let children = en
        .members
        .iter()
        .map(|member| DocItem {
            kind: DocItemKind::Variant,
            name: join_child_path(&path, &member.name),
            doc: doc_text_from_span(program, member.span),
            signature: Some(match &member.kind {
                shape_ast::ast::EnumMemberKind::Unit { .. } => member.name.clone(),
                shape_ast::ast::EnumMemberKind::Tuple(items) => format!(
                    "{}({})",
                    member.name,
                    items
                        .iter()
                        .map(format_type_annotation)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                shape_ast::ast::EnumMemberKind::Struct(fields) => format!(
                    "{} {{ {} }}",
                    member.name,
                    fields
                        .iter()
                        .map(|field| {
                            format!(
                                "{}: {}",
                                field.name,
                                format_type_annotation(&field.type_annotation)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            children: Vec::new(),
        })
        .collect();

    DocItem {
        kind: DocItemKind::Enum,
        name: path,
        doc: doc_text_from_span(program, span),
        signature: None,
        type_params: format_type_params(&en.type_params),
        params: Vec::new(),
        return_type: None,
        children,
    }
}

fn extract_trait_doc(
    program: &Program,
    path: String,
    tr: &shape_ast::ast::TraitDef,
    span: Span,
) -> DocItem {
    let mut children = Vec::new();
    for member in &tr.members {
        match member {
            TraitMember::Required(member) => {
                children.push(extract_interface_member_doc(
                    program,
                    &path,
                    member,
                    DocItemKind::Method,
                ));
            }
            TraitMember::Default(method) => {
                children.push(DocItem {
                    kind: DocItemKind::Method,
                    name: join_child_path(&path, &method.name),
                    doc: doc_text_from_span(program, method.span),
                    signature: Some(format_method_signature(method)),
                    type_params: Vec::new(),
                    params: method
                        .params
                        .iter()
                        .map(|param| DocParam {
                            name: param.simple_name().unwrap_or("_").to_string(),
                            type_name: param.type_annotation.as_ref().map(format_type_annotation),
                            description: program
                                .docs
                                .comment_for_span(method.span)
                                .and_then(|doc| doc.param_doc(param.simple_name().unwrap_or("_")))
                                .map(str::to_string),
                            default_value: None,
                        })
                        .collect(),
                    return_type: method.return_type.as_ref().map(format_type_annotation),
                    children: Vec::new(),
                });
            }
            TraitMember::AssociatedType { name, span, .. } => {
                children.push(DocItem {
                    kind: DocItemKind::AssociatedType,
                    name: join_child_path(&path, name),
                    doc: doc_text_from_span(program, *span),
                    signature: Some(format!("type {}", name)),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: None,
                    children: Vec::new(),
                });
            }
        }
    }

    DocItem {
        kind: DocItemKind::Trait,
        name: path,
        doc: doc_text_from_span(program, span),
        signature: None,
        type_params: format_type_params(&tr.type_params),
        params: Vec::new(),
        return_type: None,
        children,
    }
}

fn extract_interface_doc(
    program: &Program,
    path: String,
    interface: &shape_ast::ast::InterfaceDef,
    span: Span,
) -> DocItem {
    let children = interface
        .members
        .iter()
        .map(|member| extract_interface_member_doc(program, &path, member, DocItemKind::Method))
        .collect();

    DocItem {
        kind: DocItemKind::Interface,
        name: path,
        doc: doc_text_from_span(program, span),
        signature: None,
        type_params: format_type_params(&interface.type_params),
        params: Vec::new(),
        return_type: None,
        children,
    }
}

fn extract_interface_member_doc(
    program: &Program,
    parent_path: &str,
    member: &InterfaceMember,
    method_kind: DocItemKind,
) -> DocItem {
    match member {
        InterfaceMember::Property {
            name,
            span,
            type_annotation,
            ..
        } => DocItem {
            kind: DocItemKind::Field,
            name: join_child_path(parent_path, name),
            doc: doc_text_from_span(program, *span),
            signature: Some(format!(
                "{}: {}",
                name,
                format_type_annotation(type_annotation)
            )),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(format_type_annotation(type_annotation)),
            children: Vec::new(),
        },
        InterfaceMember::Method {
            name,
            span,
            params,
            return_type,
            ..
        } => DocItem {
            kind: method_kind,
            name: join_child_path(parent_path, name),
            doc: doc_text_from_span(program, *span),
            signature: Some(format!(
                "{}({}) -> {}",
                name,
                params
                    .iter()
                    .map(|param| {
                        let ty = format_type_annotation(&param.type_annotation);
                        match &param.name {
                            Some(name) => format!("{}: {}", name, ty),
                            None => ty,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                format_type_annotation(return_type)
            )),
            type_params: Vec::new(),
            params: params
                .iter()
                .map(|param| DocParam {
                    name: param.name.clone().unwrap_or_else(|| "_".to_string()),
                    type_name: Some(format_type_annotation(&param.type_annotation)),
                    description: program
                        .docs
                        .comment_for_span(*span)
                        .and_then(|doc| doc.param_doc(param.name.as_deref().unwrap_or("_")))
                        .map(str::to_string),
                    default_value: None,
                })
                .collect(),
            return_type: Some(format_type_annotation(return_type)),
            children: Vec::new(),
        },
        InterfaceMember::IndexSignature {
            span,
            param_name,
            param_type,
            return_type,
            ..
        } => DocItem {
            kind: method_kind,
            name: join_child_path(parent_path, &format!("[{}]", param_type)),
            doc: doc_text_from_span(program, *span),
            signature: Some(format!(
                "[{}: {}]: {}",
                param_name,
                param_type,
                format_type_annotation(return_type)
            )),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(format_type_annotation(return_type)),
            children: Vec::new(),
        },
    }
}

fn doc_text_from_span(program: &Program, span: Span) -> String {
    program
        .docs
        .comment_for_span(span)
        .map(doc_text)
        .unwrap_or_default()
}

fn doc_text(comment: &DocComment) -> String {
    if !comment.body.is_empty() {
        comment.body.clone()
    } else {
        comment.summary.clone()
    }
}

fn format_type_params(type_params: &Option<Vec<shape_ast::ast::TypeParam>>) -> Vec<String> {
    type_params
        .as_ref()
        .map(|params| params.iter().map(|tp| tp.name.clone()).collect())
        .unwrap_or_default()
}

fn format_function_signature(func: &FunctionDef) -> String {
    let type_params = format_type_params(&func.type_params);
    let type_param_suffix = if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    };
    let params = func
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("_");
            match &param.type_annotation {
                Some(ty) => format!("{}: {}", name, format_type_annotation(ty)),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_suffix = func
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", format_type_annotation(ty)))
        .unwrap_or_default();
    format!(
        "fn {}{}({}){}",
        func.name, type_param_suffix, params, return_suffix
    )
}

fn format_method_signature(method: &shape_ast::ast::MethodDef) -> String {
    let params = method
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("_");
            match &param.type_annotation {
                Some(ty) => format!("{}: {}", name, format_type_annotation(ty)),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_suffix = method
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", format_type_annotation(ty)))
        .unwrap_or_default();
    format!("fn {}({}){}", method.name, params, return_suffix)
}

fn format_builtin_signature(func: &shape_ast::ast::BuiltinFunctionDecl) -> String {
    let params = func
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("_");
            let ty = param
                .type_annotation
                .as_ref()
                .map(format_type_annotation)
                .unwrap_or_else(|| "any".to_string());
            format!("{}: {}", name, ty)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let type_params = format_type_params(&func.type_params);
    let type_param_suffix = if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    };
    format!(
        "{}{}({}) -> {}",
        func.name,
        type_param_suffix,
        params,
        format_type_annotation(&func.return_type)
    )
}

fn format_foreign_signature(func: &shape_ast::ast::ForeignFunctionDef) -> String {
    let params = func
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("_");
            match &param.type_annotation {
                Some(ty) => format!("{}: {}", name, format_type_annotation(ty)),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let type_params = format_type_params(&func.type_params);
    let type_param_suffix = if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    };
    let return_suffix = func
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", format_type_annotation(ty)))
        .unwrap_or_default();
    format!(
        "fn {} {}{}({}){}",
        func.language, func.name, type_param_suffix, params, return_suffix
    )
}

fn format_type_annotation(ta: &TypeAnnotation) -> String {
    match ta {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Array(inner) => format!("Array<{}>", format_type_annotation(inner)),
        TypeAnnotation::Tuple(items) => {
            let parts: Vec<String> = items.iter().map(format_type_annotation).collect();
            format!("[{}]", parts.join(", "))
        }
        TypeAnnotation::Generic { name, args } => {
            let parts: Vec<String> = args.iter().map(format_type_annotation).collect();
            format!("{}<{}>", name, parts.join(", "))
        }
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "null".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(bounds) => format!("dyn {}", bounds.join(" + ")),
        TypeAnnotation::Function { params, returns } => {
            let params = params
                .iter()
                .map(|param| match &param.name {
                    Some(name) => format!(
                        "{}: {}",
                        name,
                        format_type_annotation(&param.type_annotation)
                    ),
                    None => format_type_annotation(&param.type_annotation),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({}) => {}", params, format_type_annotation(returns))
        }
        TypeAnnotation::Union(items) => items
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(items) => items
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" + "),
        TypeAnnotation::Object(fields) => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|field| format!(
                    "{}: {}",
                    field.name,
                    format_type_annotation(&field.type_annotation)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn join_path(prefix: &[String], name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", prefix.join("::"), name)
    }
}

fn join_child_path(parent: &str, name: &str) -> String {
    format!("{}::{}", parent, name)
}

#[cfg(test)]
mod tests {
    use super::{DocItemKind, extract_docs_from_ast};

    #[test]
    fn extracts_function_docs_from_program_index() {
        let source = "/// Doc for hello\n/// @param value input\nfn hello(value: string) -> string { value }";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].kind, DocItemKind::Function);
        assert_eq!(docs[0].doc, "Doc for hello");
        assert_eq!(docs[0].params[0].description.as_deref(), Some("input"));
    }

    #[test]
    fn extracts_child_docs_from_program_index() {
        let source = "type Point {\n    /// X coordinate\n    x: number,\n}\n";
        let ast = shape_ast::parser::parse_program(source).expect("parse should succeed");
        let docs = extract_docs_from_ast(source, &ast);
        assert_eq!(docs[0].children.len(), 1);
        assert_eq!(docs[0].children[0].doc, "X coordinate");
    }
}
