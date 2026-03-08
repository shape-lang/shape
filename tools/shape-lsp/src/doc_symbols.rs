use crate::module_cache::ModuleCache;
use shape_ast::ast::{
    DocTargetKind, ExportItem, FunctionParameter, InterfaceMember, Item, Program, Span,
    TraitMember, TypeAnnotation, TypeParam,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DocSymbol {
    pub kind: DocTargetKind,
    pub local_path: String,
    pub qualified_path: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct DocOwner {
    pub params: Vec<String>,
    pub type_params: Vec<String>,
    pub can_have_return_doc: bool,
}

pub fn span_contains(span: Span, offset: usize) -> bool {
    !span.is_dummy() && span.start <= offset && offset <= span.end
}

pub fn qualify_doc_path(module_prefix: &str, local_path: impl AsRef<str>) -> String {
    let local_path = local_path.as_ref();
    if module_prefix.is_empty() {
        local_path.to_string()
    } else if local_path.is_empty() {
        module_prefix.to_string()
    } else {
        format!("{module_prefix}::{local_path}")
    }
}

pub fn current_module_import_path(
    cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Option<String> {
    let (cache, file_path) = (cache?, current_file?);
    let current = normalize_path(file_path);

    for module_path in cache.list_importable_modules_with_context(file_path, workspace_root) {
        let Some(resolved) = cache.resolve_import(&module_path, file_path, workspace_root) else {
            continue;
        };
        if normalize_path(&resolved) == current {
            return Some(module_path);
        }
    }

    None
}

pub fn collect_import_paths(program: &Program) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    for item in &program.items {
        if let Item::Import(import_stmt, _) = item {
            imports.insert(import_stmt.from.clone());
        }
    }
    imports
}

pub fn collect_program_doc_symbols(program: &Program, module_prefix: &str) -> Vec<DocSymbol> {
    let mut out = Vec::new();
    collect_doc_symbols_in_items(&program.items, module_prefix, &[], &mut out);
    out
}

pub fn find_doc_owner(program: &Program, target_span: Span) -> Option<DocOwner> {
    program
        .items
        .iter()
        .find_map(|item| find_doc_owner_in_item(item, target_span))
}

fn collect_doc_symbols_in_items(
    items: &[Item],
    module_prefix: &str,
    path_prefix: &[String],
    out: &mut Vec<DocSymbol>,
) {
    for item in items {
        match item {
            Item::Module(module, span) => {
                let path = join_path(path_prefix, &module.name);
                push_symbol(out, DocTargetKind::Module, module_prefix, path.clone(), *span);
                let mut next = path_prefix.to_vec();
                next.push(module.name.clone());
                collect_doc_symbols_in_items(&module.items, module_prefix, &next, out);
            }
            Item::Function(function, span) => {
                push_symbol(
                    out,
                    DocTargetKind::Function,
                    module_prefix,
                    join_path(path_prefix, &function.name),
                    *span,
                );
                push_type_params(out, module_prefix, path_prefix, &function.name, function.type_params.as_deref());
            }
            Item::AnnotationDef(annotation_def, span) => {
                push_symbol(
                    out,
                    DocTargetKind::Annotation,
                    module_prefix,
                    join_annotation_path(path_prefix, &annotation_def.name),
                    *span,
                );
            }
            Item::ForeignFunction(function, span) => {
                push_symbol(
                    out,
                    DocTargetKind::ForeignFunction,
                    module_prefix,
                    join_path(path_prefix, &function.name),
                    *span,
                );
                push_type_params(out, module_prefix, path_prefix, &function.name, function.type_params.as_deref());
            }
            Item::BuiltinFunctionDecl(function, span) => {
                push_symbol(
                    out,
                    DocTargetKind::BuiltinFunction,
                    module_prefix,
                    join_path(path_prefix, &function.name),
                    *span,
                );
                push_type_params(out, module_prefix, path_prefix, &function.name, function.type_params.as_deref());
            }
            Item::BuiltinTypeDecl(ty, span) => {
                push_symbol(
                    out,
                    DocTargetKind::BuiltinType,
                    module_prefix,
                    join_path(path_prefix, &ty.name),
                    *span,
                );
                push_type_params(out, module_prefix, path_prefix, &ty.name, ty.type_params.as_deref());
            }
            Item::TypeAlias(alias, span) => {
                push_symbol(
                    out,
                    DocTargetKind::TypeAlias,
                    module_prefix,
                    join_path(path_prefix, &alias.name),
                    *span,
                );
                push_type_params(out, module_prefix, path_prefix, &alias.name, alias.type_params.as_deref());
            }
            Item::StructType(struct_def, span) => {
                let path = join_path(path_prefix, &struct_def.name);
                push_symbol(out, DocTargetKind::Struct, module_prefix, path.clone(), *span);
                push_type_params(
                    out,
                    module_prefix,
                    path_prefix,
                    &struct_def.name,
                    struct_def.type_params.as_deref(),
                );
                for field in &struct_def.fields {
                    push_symbol(
                        out,
                        DocTargetKind::StructField,
                        module_prefix,
                        join_child_path(&path, &field.name),
                        field.span,
                    );
                }
            }
            Item::Enum(enum_def, span) => {
                let path = join_path(path_prefix, &enum_def.name);
                push_symbol(out, DocTargetKind::Enum, module_prefix, path.clone(), *span);
                push_type_params(
                    out,
                    module_prefix,
                    path_prefix,
                    &enum_def.name,
                    enum_def.type_params.as_deref(),
                );
                for member in &enum_def.members {
                    push_symbol(
                        out,
                        DocTargetKind::EnumVariant,
                        module_prefix,
                        join_child_path(&path, &member.name),
                        member.span,
                    );
                }
            }
            Item::Interface(interface, span) => {
                let path = join_path(path_prefix, &interface.name);
                push_symbol(out, DocTargetKind::Interface, module_prefix, path.clone(), *span);
                push_type_params(
                    out,
                    module_prefix,
                    path_prefix,
                    &interface.name,
                    interface.type_params.as_deref(),
                );
                for member in &interface.members {
                    push_symbol(
                        out,
                        interface_member_kind(member),
                        module_prefix,
                        join_child_path(&path, &interface_member_name(member)),
                        member.span(),
                    );
                }
            }
            Item::Trait(trait_def, span) => {
                let path = join_path(path_prefix, &trait_def.name);
                push_symbol(out, DocTargetKind::Trait, module_prefix, path.clone(), *span);
                push_type_params(
                    out,
                    module_prefix,
                    path_prefix,
                    &trait_def.name,
                    trait_def.type_params.as_deref(),
                );
                for member in &trait_def.members {
                    push_symbol(
                        out,
                        trait_member_kind(member),
                        module_prefix,
                        join_child_path(&path, &trait_member_name(member)),
                        member.span(),
                    );
                }
            }
            Item::Export(export, span) => {
                collect_export_symbols(out, module_prefix, path_prefix, export, *span);
            }
            _ => {}
        }
    }
}

fn collect_export_symbols(
    out: &mut Vec<DocSymbol>,
    module_prefix: &str,
    path_prefix: &[String],
    export: &shape_ast::ast::ExportStmt,
    span: Span,
) {
    match &export.item {
        ExportItem::Function(function) => {
            push_symbol(
                out,
                DocTargetKind::Function,
                module_prefix,
                join_path(path_prefix, &function.name),
                span,
            );
            push_type_params(out, module_prefix, path_prefix, &function.name, function.type_params.as_deref());
        }
        ExportItem::ForeignFunction(function) => {
            push_symbol(
                out,
                DocTargetKind::ForeignFunction,
                module_prefix,
                join_path(path_prefix, &function.name),
                span,
            );
            push_type_params(out, module_prefix, path_prefix, &function.name, function.type_params.as_deref());
        }
        ExportItem::TypeAlias(alias) => {
            push_symbol(
                out,
                DocTargetKind::TypeAlias,
                module_prefix,
                join_path(path_prefix, &alias.name),
                span,
            );
            push_type_params(out, module_prefix, path_prefix, &alias.name, alias.type_params.as_deref());
        }
        ExportItem::Struct(struct_def) => {
            let path = join_path(path_prefix, &struct_def.name);
            push_symbol(out, DocTargetKind::Struct, module_prefix, path.clone(), span);
            push_type_params(
                out,
                module_prefix,
                path_prefix,
                &struct_def.name,
                struct_def.type_params.as_deref(),
            );
            for field in &struct_def.fields {
                push_symbol(
                    out,
                    DocTargetKind::StructField,
                    module_prefix,
                    join_child_path(&path, &field.name),
                    field.span,
                );
            }
        }
        ExportItem::Enum(enum_def) => {
            let path = join_path(path_prefix, &enum_def.name);
            push_symbol(out, DocTargetKind::Enum, module_prefix, path.clone(), span);
            push_type_params(
                out,
                module_prefix,
                path_prefix,
                &enum_def.name,
                enum_def.type_params.as_deref(),
            );
            for member in &enum_def.members {
                push_symbol(
                    out,
                    DocTargetKind::EnumVariant,
                    module_prefix,
                    join_child_path(&path, &member.name),
                    member.span,
                );
            }
        }
        ExportItem::Interface(interface) => {
            let path = join_path(path_prefix, &interface.name);
            push_symbol(out, DocTargetKind::Interface, module_prefix, path.clone(), span);
            push_type_params(
                out,
                module_prefix,
                path_prefix,
                &interface.name,
                interface.type_params.as_deref(),
            );
            for member in &interface.members {
                push_symbol(
                    out,
                    interface_member_kind(member),
                    module_prefix,
                    join_child_path(&path, &interface_member_name(member)),
                    member.span(),
                );
            }
        }
        ExportItem::Trait(trait_def) => {
            let path = join_path(path_prefix, &trait_def.name);
            push_symbol(out, DocTargetKind::Trait, module_prefix, path.clone(), span);
            push_type_params(
                out,
                module_prefix,
                path_prefix,
                &trait_def.name,
                trait_def.type_params.as_deref(),
            );
            for member in &trait_def.members {
                push_symbol(
                    out,
                    trait_member_kind(member),
                    module_prefix,
                    join_child_path(&path, &trait_member_name(member)),
                    member.span(),
                );
            }
        }
        ExportItem::Named(_) => {}
    }
}

fn push_type_params(
    out: &mut Vec<DocSymbol>,
    module_prefix: &str,
    path_prefix: &[String],
    owner_name: &str,
    type_params: Option<&[TypeParam]>,
) {
    let owner_path = join_path(path_prefix, owner_name);
    for type_param in type_params.unwrap_or(&[]) {
        push_symbol(
            out,
            DocTargetKind::TypeParam,
            module_prefix,
            join_type_param_path(&owner_path, &type_param.name),
            type_param.span,
        );
    }
}

fn push_symbol(
    out: &mut Vec<DocSymbol>,
    kind: DocTargetKind,
    module_prefix: &str,
    local_path: String,
    span: Span,
) {
    out.push(DocSymbol {
        kind,
        qualified_path: qualify_doc_path(module_prefix, &local_path),
        local_path,
        span,
    });
}

fn interface_member_kind(member: &InterfaceMember) -> DocTargetKind {
    match member {
        InterfaceMember::Property { .. } => DocTargetKind::InterfaceProperty,
        InterfaceMember::Method { .. } => DocTargetKind::InterfaceMethod,
        InterfaceMember::IndexSignature { .. } => DocTargetKind::InterfaceIndexSignature,
    }
}

fn interface_member_name(member: &InterfaceMember) -> String {
    match member {
        InterfaceMember::Property { name, .. } | InterfaceMember::Method { name, .. } => {
            name.clone()
        }
        InterfaceMember::IndexSignature { param_type, .. } => format!("[{param_type}]"),
    }
}

fn trait_member_kind(member: &TraitMember) -> DocTargetKind {
    match member {
        TraitMember::AssociatedType { .. } => DocTargetKind::TraitAssociatedType,
        TraitMember::Required(_) | TraitMember::Default(_) => DocTargetKind::TraitMethod,
    }
}

fn trait_member_name(member: &TraitMember) -> String {
    match member {
        TraitMember::Required(member) => interface_member_name(member),
        TraitMember::Default(method) => method.name.clone(),
        TraitMember::AssociatedType { name, .. } => name.clone(),
    }
}

fn find_doc_owner_in_item(item: &Item, target_span: Span) -> Option<DocOwner> {
    match item {
        Item::Module(module, span) => {
            if *span == target_span {
                return Some(DocOwner {
                    ..Default::default()
                });
            }
            module
                .items
                .iter()
                .find_map(|child| find_doc_owner_in_item(child, target_span))
        }
        Item::Function(function, span) if *span == target_span => Some(callable_owner(
            DocTargetKind::Function,
            &function.params,
            function.type_params.as_deref(),
            function.return_type.as_ref(),
        )),
        Item::AnnotationDef(annotation_def, span) if *span == target_span => Some(DocOwner {
            params: function_param_names(&annotation_def.params),
            can_have_return_doc: false,
            ..Default::default()
        }),
        Item::ForeignFunction(function, span) if *span == target_span => Some(callable_owner(
            DocTargetKind::ForeignFunction,
            &function.params,
            function.type_params.as_deref(),
            function.return_type.as_ref(),
        )),
        Item::BuiltinFunctionDecl(function, span) if *span == target_span => Some(callable_owner(
            DocTargetKind::BuiltinFunction,
            &function.params,
            function.type_params.as_deref(),
            Some(&function.return_type),
        )),
        Item::BuiltinTypeDecl(ty, span) if *span == target_span => Some(type_owner(
            DocTargetKind::BuiltinType,
            ty.type_params.as_deref(),
        )),
        Item::TypeAlias(alias, span) if *span == target_span => Some(type_owner(
            DocTargetKind::TypeAlias,
            alias.type_params.as_deref(),
        )),
        Item::StructType(struct_def, span) if *span == target_span => Some(type_owner(
            DocTargetKind::Struct,
            struct_def.type_params.as_deref(),
        )),
        Item::Enum(enum_def, span) if *span == target_span => Some(type_owner(
            DocTargetKind::Enum,
            enum_def.type_params.as_deref(),
        )),
        Item::Interface(interface, span) if *span == target_span => Some(type_owner(
            DocTargetKind::Interface,
            interface.type_params.as_deref(),
        )),
        Item::Trait(trait_def, span) if *span == target_span => Some(type_owner(
            DocTargetKind::Trait,
            trait_def.type_params.as_deref(),
        )),
        Item::Interface(interface, _) => find_doc_owner_in_interface(interface, target_span),
        Item::Trait(trait_def, _) => find_doc_owner_in_trait(trait_def, target_span),
        Item::Export(export, span) if *span == target_span => Some(export_owner(export)),
        _ => None,
    }
}

fn find_doc_owner_in_interface(
    interface: &shape_ast::ast::InterfaceDef,
    target_span: Span,
) -> Option<DocOwner> {
    for member in &interface.members {
        if member.span() != target_span {
            continue;
        }
        return Some(match member {
            InterfaceMember::Method {
                params,
                return_type,
                ..
            } => DocOwner {
                params: interface_method_param_names(params),
                can_have_return_doc: !matches!(return_type, TypeAnnotation::Void),
                ..Default::default()
            },
            InterfaceMember::Property { .. } => DocOwner {
                ..Default::default()
            },
            InterfaceMember::IndexSignature { .. } => DocOwner {
                ..Default::default()
            },
        });
    }
    None
}

fn find_doc_owner_in_trait(
    trait_def: &shape_ast::ast::TraitDef,
    target_span: Span,
) -> Option<DocOwner> {
    for member in &trait_def.members {
        if member.span() != target_span {
            continue;
        }
        return Some(match member {
            TraitMember::Default(method) => callable_owner(
                DocTargetKind::TraitMethod,
                &method.params,
                None,
                method.return_type.as_ref(),
            ),
            TraitMember::Required(InterfaceMember::Method {
                params,
                return_type,
                ..
            }) => DocOwner {
                params: interface_method_param_names(params),
                can_have_return_doc: !matches!(return_type, TypeAnnotation::Void),
                ..Default::default()
            },
            TraitMember::Required(InterfaceMember::Property { .. })
            | TraitMember::Required(InterfaceMember::IndexSignature { .. }) => DocOwner {
                ..Default::default()
            },
            TraitMember::AssociatedType { .. } => DocOwner {
                ..Default::default()
            },
        });
    }
    None
}

fn export_owner(export: &shape_ast::ast::ExportStmt) -> DocOwner {
    match &export.item {
        ExportItem::Function(function) => callable_owner(
            DocTargetKind::Function,
            &function.params,
            function.type_params.as_deref(),
            function.return_type.as_ref(),
        ),
        ExportItem::ForeignFunction(function) => callable_owner(
            DocTargetKind::ForeignFunction,
            &function.params,
            function.type_params.as_deref(),
            function.return_type.as_ref(),
        ),
        ExportItem::TypeAlias(alias) => type_owner(DocTargetKind::TypeAlias, alias.type_params.as_deref()),
        ExportItem::Struct(struct_def) => type_owner(DocTargetKind::Struct, struct_def.type_params.as_deref()),
        ExportItem::Enum(enum_def) => type_owner(DocTargetKind::Enum, enum_def.type_params.as_deref()),
        ExportItem::Interface(interface) => {
            type_owner(DocTargetKind::Interface, interface.type_params.as_deref())
        }
        ExportItem::Trait(trait_def) => type_owner(DocTargetKind::Trait, trait_def.type_params.as_deref()),
        ExportItem::Named(_) => DocOwner::default(),
    }
}

fn callable_owner(
    _kind: DocTargetKind,
    params: &[FunctionParameter],
    type_params: Option<&[TypeParam]>,
    return_type: Option<&TypeAnnotation>,
) -> DocOwner {
    DocOwner {
        params: function_param_names(params),
        type_params: type_param_names(type_params),
        can_have_return_doc: !matches!(return_type, Some(TypeAnnotation::Void)),
    }
}

fn type_owner(_kind: DocTargetKind, type_params: Option<&[TypeParam]>) -> DocOwner {
    DocOwner {
        type_params: type_param_names(type_params),
        ..Default::default()
    }
}

pub fn type_param_names(type_params: Option<&[TypeParam]>) -> Vec<String> {
    type_params
        .unwrap_or(&[])
        .iter()
        .map(|param| param.name.clone())
        .collect()
}

pub fn function_param_names(params: &[FunctionParameter]) -> Vec<String> {
    let mut names = Vec::new();
    for param in params {
        names.extend(param.get_identifiers());
    }
    names.sort();
    names.dedup();
    names
}

fn interface_method_param_names(params: &[shape_ast::ast::FunctionParam]) -> Vec<String> {
    let mut names = params
        .iter()
        .filter_map(|param| param.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
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
    format!("{parent}::{name}")
}

fn join_type_param_path(parent: &str, name: &str) -> String {
    format!("{parent}::<{name}>")
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn collects_member_symbols_with_qualified_paths() {
        let program = parse_program(
            "type Point { /// x\n x: number }\ntrait Drawable { /// draw\n draw(): void }\n",
        )
        .expect("program");
        let symbols = collect_program_doc_symbols(&program, "pkg::math");
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.qualified_path == "pkg::math::Point::x")
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.qualified_path == "pkg::math::Drawable::draw")
        );
    }

    #[test]
    fn collects_annotation_symbols_with_canonical_paths() {
        let program = parse_program("/// Trace execution.\nannotation trace() {}\n").expect("program");
        let symbols = collect_program_doc_symbols(&program, "pkg::debug");
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.qualified_path == "pkg::debug::@trace")
        );
    }
}
