use super::*;
use shape_ast::ast::{
    DestructurePattern, ExportItem, ExtendStatement, ForInit, FunctionDef, ImplBlock, Item,
    MethodDef, Pattern, PatternConstructorFields, Statement,
};
use std::collections::HashSet;

impl BytecodeCompiler {
    pub(super) fn module_local_function_names(items: &[Item]) -> HashSet<String> {
        let mut names = HashSet::new();
        for item in items {
            match item {
                Item::Function(func, _) => {
                    names.insert(func.name.clone());
                }
                Item::Export(export, _) => match &export.item {
                    ExportItem::Function(func) => {
                        names.insert(func.name.clone());
                    }
                    ExportItem::BuiltinFunction(_)
                    | ExportItem::BuiltinType(_)
                    | ExportItem::TypeAlias(_)
                    | ExportItem::Named(_)
                    | ExportItem::Enum(_)
                    | ExportItem::Struct(_)
                    | ExportItem::Trait(_)
                    | ExportItem::Annotation(_)
                    | ExportItem::ForeignFunction(_) => {}
                },
                Item::Import(_, _)
                | Item::Module(_, _)
                | Item::TypeAlias(_, _)
                | Item::Trait(_, _)
                | Item::Enum(_, _)
                | Item::Extend(_, _)
                | Item::Impl(_, _)
                | Item::Query(_, _)
                | Item::VariableDecl(_, _)
                | Item::Assignment(_, _)
                | Item::Expression(_, _)
                | Item::Stream(_, _)
                | Item::Test(_, _)
                | Item::Optimize(_, _)
                | Item::AnnotationDef(_, _)
                | Item::StructType(_, _)
                | Item::DataSource(_, _)
                | Item::QueryDecl(_, _)
                | Item::Statement(_, _)
                | Item::Comptime(_, _)
                | Item::BuiltinTypeDecl(_, _)
                | Item::BuiltinFunctionDecl(_, _)
                | Item::ForeignFunction(_, _) => {}
            }
        }
        names
    }

    pub(super) fn qualify_module_item_with_local_function_calls(
        &self,
        item: &Item,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) -> Result<Item> {
        let mut qualified = self.qualify_module_item(item, module_path)?;
        Self::qualify_local_calls_in_item(&mut qualified, module_path, local_functions);
        Ok(qualified)
    }

    fn qualify_local_calls_in_item(
        item: &mut Item,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        match item {
            Item::Function(func, _) => {
                Self::qualify_local_calls_in_function(func, module_path, local_functions);
            }
            Item::Export(export, _) => match &mut export.item {
                ExportItem::Function(func) => {
                    Self::qualify_local_calls_in_function(func, module_path, local_functions);
                }
                ExportItem::BuiltinFunction(_)
                | ExportItem::BuiltinType(_)
                | ExportItem::TypeAlias(_)
                | ExportItem::Named(_)
                | ExportItem::Enum(_)
                | ExportItem::Struct(_)
                | ExportItem::Trait(_)
                | ExportItem::Annotation(_)
                | ExportItem::ForeignFunction(_) => {}
            },
            Item::Extend(extend, _) => {
                Self::qualify_local_calls_in_extend(extend, module_path, local_functions);
            }
            Item::Impl(impl_block, _) => {
                Self::qualify_local_calls_in_impl(impl_block, module_path, local_functions);
            }
            // This helper is a module-function call binder. Other item shapes
            // either have no function body here or are compiled by separate item
            // paths after the existing module qualification pass.
            Item::Import(_, _)
            | Item::Module(_, _)
            | Item::TypeAlias(_, _)
            | Item::Trait(_, _)
            | Item::Enum(_, _)
            | Item::Query(_, _)
            | Item::VariableDecl(_, _)
            | Item::Assignment(_, _)
            | Item::Expression(_, _)
            | Item::Stream(_, _)
            | Item::Test(_, _)
            | Item::Optimize(_, _)
            | Item::AnnotationDef(_, _)
            | Item::StructType(_, _)
            | Item::DataSource(_, _)
            | Item::QueryDecl(_, _)
            | Item::Statement(_, _)
            | Item::Comptime(_, _)
            | Item::BuiltinTypeDecl(_, _)
            | Item::BuiltinFunctionDecl(_, _)
            | Item::ForeignFunction(_, _) => {}
        }
    }

    fn qualify_local_calls_in_function(
        func: &mut FunctionDef,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        let mut annotation_shadowed = HashSet::new();
        Self::qualify_local_calls_in_annotations(
            &mut func.annotations,
            module_path,
            local_functions,
            &mut annotation_shadowed,
        );

        let mut shadowed = HashSet::new();
        for param in &mut func.params {
            if let Some(default_value) = param.default_value.as_mut() {
                Self::qualify_local_calls_in_expr(
                    default_value,
                    module_path,
                    local_functions,
                    &mut shadowed,
                );
            }
            Self::bind_pattern_names(&param.pattern, &mut shadowed);
        }
        Self::qualify_local_calls_in_statements(
            &mut func.body,
            module_path,
            local_functions,
            &mut shadowed,
        );
    }

    fn qualify_local_calls_in_extend(
        extend: &mut ExtendStatement,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        for method in &mut extend.methods {
            Self::qualify_local_calls_in_method(method, module_path, local_functions);
        }
    }

    fn qualify_local_calls_in_impl(
        impl_block: &mut ImplBlock,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        for method in &mut impl_block.methods {
            Self::qualify_local_calls_in_method(method, module_path, local_functions);
        }
    }

    fn qualify_local_calls_in_method(
        method: &mut MethodDef,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        let mut shadowed = HashSet::new();
        Self::qualify_local_calls_in_annotations(
            &mut method.annotations,
            module_path,
            local_functions,
            &mut shadowed,
        );
        for param in &mut method.params {
            if let Some(default_value) = param.default_value.as_mut() {
                Self::qualify_local_calls_in_expr(
                    default_value,
                    module_path,
                    local_functions,
                    &mut shadowed,
                );
            }
            Self::bind_pattern_names(&param.pattern, &mut shadowed);
        }
        if let Some(when_clause) = method.when_clause.as_mut() {
            Self::qualify_local_calls_in_expr(
                when_clause,
                module_path,
                local_functions,
                &mut shadowed,
            );
        }
        Self::qualify_local_calls_in_statements(
            &mut method.body,
            module_path,
            local_functions,
            &mut shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_statements(
        statements: &mut [Statement],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for statement in statements {
            match statement {
                Statement::VariableDecl(decl, _) => {
                    if let Some(value) = decl.value.as_mut() {
                        Self::qualify_local_calls_in_expr(
                            value,
                            module_path,
                            local_functions,
                            shadowed,
                        );
                    }
                    Self::bind_pattern_names(&decl.pattern, shadowed);
                }
                Statement::Assignment(assign, _) => {
                    Self::qualify_local_calls_in_expr(
                        &mut assign.value,
                        module_path,
                        local_functions,
                        shadowed,
                    );
                }
                Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                    Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
                }
                Statement::If(if_stmt, _) => {
                    Self::qualify_local_calls_in_expr(
                        &mut if_stmt.condition,
                        module_path,
                        local_functions,
                        shadowed,
                    );
                    let mut then_shadowed = shadowed.clone();
                    Self::qualify_local_calls_in_statements(
                        &mut if_stmt.then_body,
                        module_path,
                        local_functions,
                        &mut then_shadowed,
                    );
                    if let Some(else_body) = if_stmt.else_body.as_mut() {
                        let mut else_shadowed = shadowed.clone();
                        Self::qualify_local_calls_in_statements(
                            else_body,
                            module_path,
                            local_functions,
                            &mut else_shadowed,
                        );
                    }
                }
                Statement::While(while_stmt, _) => {
                    Self::qualify_local_calls_in_expr(
                        &mut while_stmt.condition,
                        module_path,
                        local_functions,
                        shadowed,
                    );
                    let mut body_shadowed = shadowed.clone();
                    Self::qualify_local_calls_in_statements(
                        &mut while_stmt.body,
                        module_path,
                        local_functions,
                        &mut body_shadowed,
                    );
                }
                Statement::For(for_stmt, _) => {
                    let mut body_shadowed = shadowed.clone();
                    match &mut for_stmt.init {
                        ForInit::ForIn { pattern, iter } => {
                            Self::qualify_local_calls_in_expr(
                                iter,
                                module_path,
                                local_functions,
                                shadowed,
                            );
                            Self::bind_pattern_names(pattern, &mut body_shadowed);
                        }
                        ForInit::ForC {
                            init,
                            condition,
                            update,
                        } => {
                            Self::qualify_local_calls_in_statements(
                                std::slice::from_mut(init.as_mut()),
                                module_path,
                                local_functions,
                                &mut body_shadowed,
                            );
                            Self::qualify_local_calls_in_expr(
                                condition,
                                module_path,
                                local_functions,
                                &mut body_shadowed,
                            );
                            Self::qualify_local_calls_in_expr(
                                update,
                                module_path,
                                local_functions,
                                &mut body_shadowed,
                            );
                        }
                    }
                    Self::qualify_local_calls_in_statements(
                        &mut for_stmt.body,
                        module_path,
                        local_functions,
                        &mut body_shadowed,
                    );
                }
                Statement::Extend(extend, _) => {
                    Self::qualify_local_calls_in_extend(extend, module_path, local_functions);
                }
                Statement::SetParamValue { expression, .. }
                | Statement::SetReturnExpr { expression, .. }
                | Statement::ReplaceBodyExpr { expression, .. }
                | Statement::ReplaceModuleExpr { expression, .. }
                | Statement::ExtendItemsExpr { expression, .. } => {
                    Self::qualify_local_calls_in_expr(
                        expression,
                        module_path,
                        local_functions,
                        shadowed,
                    );
                }
                Statement::ReplaceBody { body, .. } => {
                    let mut body_shadowed = shadowed.clone();
                    Self::qualify_local_calls_in_statements(
                        body,
                        module_path,
                        local_functions,
                        &mut body_shadowed,
                    );
                }
                // Terminal statements and type-only comptime directives have no
                // child expressions to qualify.
                Statement::Return(None, _)
                | Statement::Break(_)
                | Statement::Continue(_)
                | Statement::RemoveTarget(_)
                | Statement::SetParamType { .. }
                | Statement::SetReturnType { .. } => {}
            }
        }
    }

    pub(super) fn should_qualify_local_call(
        name: &str,
        local_functions: &HashSet<String>,
        shadowed: &HashSet<String>,
    ) -> bool {
        !name.contains("::") && local_functions.contains(name) && !shadowed.contains(name)
    }

    pub(super) fn bind_pattern_names(pattern: &DestructurePattern, out: &mut HashSet<String>) {
        for (name, _) in pattern.get_bindings() {
            out.insert(name);
        }
    }

    pub(super) fn bind_match_pattern_names(pattern: &Pattern, out: &mut HashSet<String>) {
        match pattern {
            Pattern::Identifier { name, .. } | Pattern::Typed { name, .. } => {
                out.insert(name.clone());
            }
            Pattern::Array(items) => {
                for item in items {
                    Self::bind_match_pattern_names(item, out);
                }
            }
            Pattern::Object(fields) => {
                for (_, item) in fields {
                    Self::bind_match_pattern_names(item, out);
                }
            }
            Pattern::Constructor { fields, .. } => match fields {
                PatternConstructorFields::Unit => {}
                PatternConstructorFields::Tuple(items) => {
                    for item in items {
                        Self::bind_match_pattern_names(item, out);
                    }
                }
                PatternConstructorFields::Struct(fields) => {
                    for (_, item) in fields {
                        Self::bind_match_pattern_names(item, out);
                    }
                }
            },
            Pattern::Literal(_) | Pattern::Wildcard => {}
        }
    }
}
