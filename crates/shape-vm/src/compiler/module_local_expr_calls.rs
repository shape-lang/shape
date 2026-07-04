use super::*;
use shape_ast::ast::Expr;
use std::collections::HashSet;

impl BytecodeCompiler {
    pub(super) fn qualify_local_calls_in_expr(
        expr: &mut Expr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match expr {
            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                ..
            } => {
                Self::qualify_local_function_call(
                    name,
                    const_args,
                    args,
                    named_args,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::QualifiedFunctionCall {
                const_args,
                args,
                named_args,
                ..
            } => {
                Self::qualify_local_call_arguments(
                    const_args,
                    args,
                    named_args,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                Self::qualify_local_method_call(
                    receiver,
                    args,
                    named_args,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::DataRef(data_ref, _) => {
                Self::qualify_local_calls_in_data_index(
                    &mut data_ref.index,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::DataRelativeAccess {
                reference, index, ..
            } => {
                Self::qualify_local_calls_in_expr(
                    reference,
                    module_path,
                    local_functions,
                    shadowed,
                );
                Self::qualify_local_calls_in_data_index(
                    index,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::PropertyAccess { object, .. } => {
                Self::qualify_local_calls_in_expr(object, module_path, local_functions, shadowed);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::qualify_local_calls_in_expr(object, module_path, local_functions, shadowed);
                Self::qualify_local_calls_in_expr(index, module_path, local_functions, shadowed);
                Self::qualify_local_calls_in_optional_box(
                    end_index,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::qualify_local_calls_in_expr(left, module_path, local_functions, shadowed);
                Self::qualify_local_calls_in_expr(right, module_path, local_functions, shadowed);
            }
            Expr::UnaryOp { operand, .. } => {
                Self::qualify_local_calls_in_expr(operand, module_path, local_functions, shadowed);
            }
            Expr::EnumConstructor { payload, .. } => {
                Self::qualify_local_calls_in_enum_payload(
                    payload,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::qualify_local_conditional_expr(
                    condition,
                    then_expr,
                    else_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Object(entries, _) => {
                Self::qualify_local_calls_in_object_entries(
                    entries,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Array(items, _) => {
                Self::qualify_local_calls_in_exprs(items, module_path, local_functions, shadowed);
            }
            Expr::ListComprehension(comp, _) => {
                Self::qualify_local_calls_in_list_comprehension(
                    comp,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Block(block, _) => {
                Self::qualify_local_calls_in_block_items(
                    &mut block.items,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
                if let Some(overrides) = meta_param_overrides.as_mut() {
                    for value in overrides.values_mut() {
                        Self::qualify_local_calls_in_expr(
                            value,
                            module_path,
                            local_functions,
                            shadowed,
                        );
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => {
                Self::qualify_local_calls_in_expr(expr, module_path, local_functions, shadowed);
            }
            Expr::FunctionExpr { params, body, .. } => {
                Self::qualify_local_calls_in_function_expr(
                    params,
                    body,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Spread(inner, _)
            | Expr::Await(inner, _)
            | Expr::AsyncScope(inner, _)
            | Expr::TryOperator(inner, _)
            | Expr::UsingImpl { expr: inner, .. }
            | Expr::Reference { expr: inner, .. }
            | Expr::TimeframeContext { expr: inner, .. } => {
                Self::qualify_local_calls_in_expr(inner, module_path, local_functions, shadowed);
            }
            Expr::If(if_expr, _) => {
                Self::qualify_local_calls_in_if_expr(
                    if_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::While(while_expr, _) => {
                Self::qualify_local_calls_in_while_expr(
                    while_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::For(for_expr, _) => {
                Self::qualify_local_calls_in_for_expr(
                    for_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Loop(loop_expr, _) => {
                Self::qualify_local_calls_in_loop_expr(
                    loop_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Let(let_expr, _) => {
                Self::qualify_local_calls_in_let_expr(
                    let_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Assign(assign, _) => {
                Self::qualify_local_calls_in_expr(
                    &mut assign.target,
                    module_path,
                    local_functions,
                    shadowed,
                );
                Self::qualify_local_calls_in_expr(
                    &mut assign.value,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Break(Some(inner), _) | Expr::Return(Some(inner), _) => {
                Self::qualify_local_calls_in_expr(inner, module_path, local_functions, shadowed);
            }
            Expr::Match(match_expr, _) => {
                Self::qualify_local_calls_in_match_expr(
                    match_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Range { start, end, .. } => {
                Self::qualify_local_calls_in_optional_box(
                    start,
                    module_path,
                    local_functions,
                    shadowed,
                );
                Self::qualify_local_calls_in_optional_box(
                    end,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::SimulationCall { params, .. } | Expr::StructLiteral { fields: params, .. } => {
                Self::qualify_local_calls_in_named_exprs(
                    params,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::WindowExpr(window_expr, _) => {
                Self::qualify_local_calls_in_window_expr(
                    window_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::FromQuery(from_query, _) => {
                Self::qualify_local_calls_in_from_query(
                    from_query,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Join(join_expr, _) => {
                Self::qualify_local_calls_in_join_expr(
                    join_expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::Annotated {
                annotation, target, ..
            } => {
                Self::qualify_local_calls_in_annotation(
                    annotation,
                    module_path,
                    local_functions,
                    shadowed,
                );
                Self::qualify_local_calls_in_expr(target, module_path, local_functions, shadowed);
            }
            Expr::AsyncLet(async_let, _) => {
                Self::qualify_local_calls_in_expr(
                    &mut async_let.expr,
                    module_path,
                    local_functions,
                    shadowed,
                );
                shadowed.insert(async_let.name.clone());
            }
            Expr::Comptime(statements, _) => {
                let mut block_shadowed = shadowed.clone();
                Self::qualify_local_calls_in_statements(
                    statements,
                    module_path,
                    local_functions,
                    &mut block_shadowed,
                );
            }
            Expr::ComptimeFor(comptime_for, _) => {
                Self::qualify_local_calls_in_comptime_for_expr(
                    comptime_for,
                    module_path,
                    local_functions,
                    shadowed,
                );
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    Self::qualify_local_calls_in_exprs(row, module_path, local_functions, shadowed);
                }
            }
            // Terminal expression variants have no child expressions.
            Expr::Literal(_, _)
            | Expr::Identifier(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::Duration(_, _)
            | Expr::Continue(_)
            | Expr::Break(None, _)
            | Expr::Return(None, _)
            | Expr::Unit(_) => {}
        }
    }
}
