use super::*;
use shape_ast::ast::{
    ComptimeForExpr, Expr, ForExpr, FunctionParameter, IfExpr, JoinExpr, LetExpr,
    ListComprehension, LoopExpr, MatchExpr, Statement, WhileExpr,
};
use std::collections::HashSet;

impl BytecodeCompiler {
    pub(super) fn qualify_local_conditional_expr(
        condition: &mut Box<Expr>,
        then_expr: &mut Box<Expr>,
        else_expr: &mut Option<Box<Expr>>,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(condition, module_path, local_functions, shadowed);
        let mut then_shadowed = shadowed.clone();
        Self::qualify_local_calls_in_expr(
            then_expr,
            module_path,
            local_functions,
            &mut then_shadowed,
        );
        if let Some(else_expr) = else_expr.as_mut() {
            let mut else_shadowed = shadowed.clone();
            Self::qualify_local_calls_in_expr(
                else_expr,
                module_path,
                local_functions,
                &mut else_shadowed,
            );
        }
    }

    pub(super) fn qualify_local_calls_in_list_comprehension(
        comp: &mut ListComprehension,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        let mut comp_shadowed = shadowed.clone();
        for clause in &mut comp.clauses {
            Self::qualify_local_calls_in_expr(
                &mut clause.iterable,
                module_path,
                local_functions,
                &mut comp_shadowed,
            );
            Self::bind_pattern_names(&clause.pattern, &mut comp_shadowed);
            Self::qualify_local_calls_in_optional_box(
                &mut clause.filter,
                module_path,
                local_functions,
                &mut comp_shadowed,
            );
        }
        Self::qualify_local_calls_in_expr(
            &mut comp.element,
            module_path,
            local_functions,
            &mut comp_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_function_expr(
        params: &mut [FunctionParameter],
        body: &mut [Statement],
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        let mut closure_shadowed = shadowed.clone();
        for param in params {
            if let Some(default_value) = param.default_value.as_mut() {
                Self::qualify_local_calls_in_expr(
                    default_value,
                    module_path,
                    local_functions,
                    &mut closure_shadowed,
                );
            }
            Self::bind_pattern_names(&param.pattern, &mut closure_shadowed);
        }
        Self::qualify_local_calls_in_statements(
            body,
            module_path,
            local_functions,
            &mut closure_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_if_expr(
        if_expr: &mut IfExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut if_expr.condition,
            module_path,
            local_functions,
            shadowed,
        );
        let mut then_shadowed = shadowed.clone();
        Self::qualify_local_calls_in_expr(
            &mut if_expr.then_branch,
            module_path,
            local_functions,
            &mut then_shadowed,
        );
        if let Some(else_branch) = if_expr.else_branch.as_mut() {
            let mut else_shadowed = shadowed.clone();
            Self::qualify_local_calls_in_expr(
                else_branch,
                module_path,
                local_functions,
                &mut else_shadowed,
            );
        }
    }

    pub(super) fn qualify_local_calls_in_while_expr(
        while_expr: &mut WhileExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut while_expr.condition,
            module_path,
            local_functions,
            shadowed,
        );
        let mut body_shadowed = shadowed.clone();
        Self::qualify_local_calls_in_expr(
            &mut while_expr.body,
            module_path,
            local_functions,
            &mut body_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_for_expr(
        for_expr: &mut ForExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut for_expr.iterable,
            module_path,
            local_functions,
            shadowed,
        );
        let mut body_shadowed = shadowed.clone();
        Self::bind_match_pattern_names(&for_expr.pattern, &mut body_shadowed);
        Self::qualify_local_calls_in_expr(
            &mut for_expr.body,
            module_path,
            local_functions,
            &mut body_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_loop_expr(
        loop_expr: &mut LoopExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        let mut body_shadowed = shadowed.clone();
        Self::qualify_local_calls_in_expr(
            &mut loop_expr.body,
            module_path,
            local_functions,
            &mut body_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_let_expr(
        let_expr: &mut LetExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_optional_box(
            &mut let_expr.value,
            module_path,
            local_functions,
            shadowed,
        );
        let mut body_shadowed = shadowed.clone();
        Self::bind_match_pattern_names(&let_expr.pattern, &mut body_shadowed);
        Self::qualify_local_calls_in_expr(
            &mut let_expr.body,
            module_path,
            local_functions,
            &mut body_shadowed,
        );
    }

    pub(super) fn qualify_local_calls_in_match_expr(
        match_expr: &mut MatchExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut match_expr.scrutinee,
            module_path,
            local_functions,
            shadowed,
        );
        for arm in &mut match_expr.arms {
            let mut arm_shadowed = shadowed.clone();
            Self::bind_match_pattern_names(&arm.pattern, &mut arm_shadowed);
            Self::qualify_local_calls_in_optional_box(
                &mut arm.guard,
                module_path,
                local_functions,
                &mut arm_shadowed,
            );
            Self::qualify_local_calls_in_expr(
                &mut arm.body,
                module_path,
                local_functions,
                &mut arm_shadowed,
            );
        }
    }

    pub(super) fn qualify_local_calls_in_join_expr(
        join_expr: &mut JoinExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        for branch in &mut join_expr.branches {
            Self::qualify_local_calls_in_annotations(
                &mut branch.annotations,
                module_path,
                local_functions,
                shadowed,
            );
            Self::qualify_local_calls_in_expr(
                &mut branch.expr,
                module_path,
                local_functions,
                shadowed,
            );
        }
    }

    pub(super) fn qualify_local_calls_in_comptime_for_expr(
        comptime_for: &mut ComptimeForExpr,
        module_path: &str,
        local_functions: &HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        Self::qualify_local_calls_in_expr(
            &mut comptime_for.iterable,
            module_path,
            local_functions,
            shadowed,
        );
        let mut body_shadowed = shadowed.clone();
        body_shadowed.insert(comptime_for.variable.clone());
        Self::qualify_local_calls_in_statements(
            &mut comptime_for.body,
            module_path,
            local_functions,
            &mut body_shadowed,
        );
    }
}
