use super::*;
use shape_ast::ast::{
    Annotation, BlockItem, DataIndex, EnumConstructorPayload, Expr, ObjectEntry, QueryClause,
    WindowExpr, WindowFunction,
};
use std::collections::HashSet;

impl BytecodeCompiler {
    pub(super) fn qualify_local_function_call(
        name: &mut String,
        const_args: &mut [Expr],
        args: &mut [Expr],
        named_args: &mut [(String, Expr)],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_call_arguments(
            const_args,
            args,
            named_args,
            module_path,
            local_functions,
            shadowed,
        );
        if Self::should_qualify_local_call(name.as_str(), local_functions, shadowed) {
            *name = Self::qualify_module_symbol(module_path, name);
        }
    }

    pub(super) fn qualify_local_call_arguments(
        const_args: &mut [Expr],
        args: &mut [Expr],
        named_args: &mut [(String, Expr)],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_exprs(const_args, module_path, local_functions, shadowed);
        Self::qualify_local_calls_in_exprs(args, module_path, local_functions, shadowed);
        Self::qualify_local_calls_in_named_exprs(
            named_args,
            module_path,
            local_functions,
            shadowed,
        );
    }

    pub(super) fn qualify_local_method_call(
        receiver: &mut Box<Expr>,
        args: &mut [Expr],
        named_args: &mut [(String, Expr)],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(receiver, module_path, local_functions, shadowed);
        Self::qualify_local_calls_in_exprs(args, module_path, local_functions, shadowed);
        Self::qualify_local_calls_in_named_exprs(
            named_args,
            module_path,
            local_functions,
            shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_object_entries(
        entries: &mut [ObjectEntry],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for entry in entries {
            match entry {
                ObjectEntry::Field { value, .. } | ObjectEntry::Spread(value) => {
                    Self::qualify_local_calls_in_expr(
                        value,
                        module_path,
                        local_functions,
                        shadowed,
                    );
                }
            }
        }
    }

    pub(super) fn qualify_local_calls_in_annotations(
        annotations: &mut [Annotation],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for annotation in annotations {
            Self::qualify_local_calls_in_annotation(
                annotation,
                module_path,
                local_functions,
                shadowed,
            );
        }
    }

    pub(super) fn qualify_local_calls_in_annotation(
        annotation: &mut Annotation,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_exprs(
            &mut annotation.args,
            module_path,
            local_functions,
            shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_exprs(
        exprs: &mut [Expr],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for expr in exprs {
            Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
        }
    }

    pub(super) fn qualify_local_calls_in_named_exprs(
        exprs: &mut [(String, Expr)],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for (_, expr) in exprs {
            Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
        }
    }

    pub(super) fn qualify_local_calls_in_optional_box(
        expr: &mut Option<Box<Expr>>,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        if let Some(expr) = expr.as_mut() {
            Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
        }
    }

    pub(super) fn qualify_local_calls_in_data_index(
        index: &mut DataIndex,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match index {
            DataIndex::Expression(expr) => {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
            }
            DataIndex::ExpressionRange(start, end) => {
                Self::qualify_local_calls_in_expr(start, module_path, local_functions, shadowed);
                Self::qualify_local_calls_in_expr(end, module_path, local_functions, shadowed);
            }
            DataIndex::Single(_) | DataIndex::Range(_, _) => {}
        }
    }

    pub(super) fn qualify_local_calls_in_enum_payload(
        payload: &mut EnumConstructorPayload,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match payload {
            EnumConstructorPayload::Unit => {}
            EnumConstructorPayload::Tuple(values) => {
                Self::qualify_local_calls_in_exprs(values, module_path, local_functions, shadowed);
            }
            EnumConstructorPayload::Struct(fields) => {
                Self::qualify_local_calls_in_named_exprs(
                    fields,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
        }
    }

    pub(super) fn qualify_local_calls_in_block_items(
        items: &mut [BlockItem],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        let mut block_shadowed = shadowed.clone();
        for item in items {
            match item {
                BlockItem::VariableDecl(decl) => {
                    if let Some(value) = decl.value.as_mut() {
                        Self::qualify_local_calls_in_expr(
                            value,
                            module_path,
                            local_functions,
                            &mut block_shadowed,
                        );
                    }
                    Self::bind_pattern_names(&decl.pattern, &mut block_shadowed);
                }
                BlockItem::Assignment(assign) => {
                    Self::qualify_local_calls_in_expr(
                        &mut assign.value,
                        module_path,
                        local_functions,
                        &mut block_shadowed,
                    );
                }
                BlockItem::Statement(stmt) => {
                    Self::qualify_local_calls_in_statements(
                        std::slice::from_mut(stmt),
                        module_path,
                        local_functions,
                        &mut block_shadowed,
                    );
                }
                BlockItem::Expression(expr) => {
                    Self::qualify_local_calls_in_expr(
                        expr,
                        module_path,
                        local_functions,
                        &mut block_shadowed,
                    );
                }
            }
        }
    }

    pub(super) fn qualify_local_calls_in_window_expr(
        window_expr: &mut WindowExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match &mut window_expr.function {
            WindowFunction::Lag { expr, default, .. }
            | WindowFunction::Lead { expr, default, .. } => {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
                Self::qualify_local_calls_in_optional_box(
                    default,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            WindowFunction::FirstValue(expr)
            | WindowFunction::LastValue(expr)
            | WindowFunction::NthValue(expr, _)
            | WindowFunction::Sum(expr)
            | WindowFunction::Avg(expr)
            | WindowFunction::Min(expr)
            | WindowFunction::Max(expr)
            | WindowFunction::Count(Some(expr)) => {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
            }
            WindowFunction::Count(None)
            | WindowFunction::RowNumber
            | WindowFunction::Rank
            | WindowFunction::DenseRank
            | WindowFunction::Ntile(_) => {}
        }

        Self::qualify_local_calls_in_exprs(
            &mut window_expr.over.partition_by,
            module_path,
            local_functions,
            shadowed,
        );
        if let Some(order_by) = window_expr.over.order_by.as_mut() {
            for (expr, _) in &mut order_by.columns {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
            }
        }
    }

    pub(super) fn qualify_local_calls_in_from_query(
        from_query: &mut shape_ast::ast::FromQueryExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut from_query.source,
            module_path,
            local_functions,
            shadowed,
        );
        let mut query_shadowed = shadowed.clone();
        query_shadowed.insert(from_query.variable.clone());

        for clause in &mut from_query.clauses {
            match clause {
                QueryClause::Where(expr) => {
                    Self::qualify_local_calls_in_expr(
                        expr,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                }
                QueryClause::OrderBy(specs) => {
                    for spec in specs {
                        Self::qualify_local_calls_in_expr(
                            &mut spec.key,
                            module_path,
                            local_functions,
                            &mut query_shadowed,
                        );
                    }
                }
                QueryClause::GroupBy {
                    element,
                    key,
                    into_var,
                } => {
                    Self::qualify_local_calls_in_expr(
                        element,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                    Self::qualify_local_calls_in_expr(
                        key,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                    if let Some(into_var) = into_var {
                        query_shadowed.insert(into_var.clone());
                    }
                }
                QueryClause::Join {
                    variable,
                    source,
                    left_key,
                    right_key,
                    into_var,
                } => {
                    Self::qualify_local_calls_in_expr(
                        source,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                    Self::qualify_local_calls_in_expr(
                        left_key,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                    let mut right_shadowed = query_shadowed.clone();
                    right_shadowed.insert(variable.clone());
                    Self::qualify_local_calls_in_expr(
                        right_key,
                        module_path,
                        local_functions,
                        &mut right_shadowed,
                    );
                    query_shadowed.insert(variable.clone());
                    if let Some(into_var) = into_var {
                        query_shadowed.insert(into_var.clone());
                    }
                }
                QueryClause::Let { variable, value } => {
                    Self::qualify_local_calls_in_expr(
                        value,
                        module_path,
                        local_functions,
                        &mut query_shadowed,
                    );
                    query_shadowed.insert(variable.clone());
                }
            }
        }

        Self::qualify_local_calls_in_expr(
            &mut from_query.select,
            module_path,
            local_functions,
            &mut query_shadowed,
        );
    }
}
