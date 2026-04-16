use super::*;
use shape_value::ValueWordExt;

impl BytecodeCompiler {
    pub(super) fn infer_reference_params_from_types(
        program: &Program,
        inferred_types: &HashMap<String, Type>,
    ) -> HashMap<String, Vec<bool>> {
        let funcs = Self::collect_program_functions(program);
        let mut inferred = HashMap::new();

        for (name, func) in funcs {
            let mut inferred_flags = vec![false; func.params.len()];
            let Some(Type::Function { params, .. }) = inferred_types.get(&name) else {
                inferred.insert(name, inferred_flags);
                continue;
            };

            for (idx, param) in func.params.iter().enumerate() {
                if param.type_annotation.is_some()
                    || param.is_reference
                    || param.simple_name().is_none()
                {
                    continue;
                }
                if let Some(inferred_param_ty) = params.get(idx)
                    && Self::type_is_heap_like(inferred_param_ty)
                {
                    inferred_flags[idx] = true;
                }
            }
            inferred.insert(name, inferred_flags);
        }

        inferred
    }

    pub(super) fn analyze_statement_for_ref_mutation(
        stmt: &shape_ast::ast::Statement,
        caller_name: &str,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
        callee_ref_params: &HashMap<String, Vec<bool>>,
        direct_mutates: &mut [bool],
        edges: &mut Vec<(String, usize, String, usize)>,
    ) {
        use shape_ast::ast::{ForInit, Statement};

        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::analyze_expr_for_ref_mutation(
                    expr,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    Self::analyze_expr_for_ref_mutation(
                        value,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier()
                    && let Some(&idx) = param_index_by_name.get(name)
                    && caller_ref_params.get(idx).copied().unwrap_or(false)
                {
                    direct_mutates[idx] = true;
                }
                Self::analyze_expr_for_ref_mutation(
                    &assign.value,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::If(if_stmt, _) => {
                Self::analyze_expr_for_ref_mutation(
                    &if_stmt.condition,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
                for stmt in &if_stmt.then_body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        Self::analyze_statement_for_ref_mutation(
                            stmt,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
            }
            Statement::While(while_loop, _) => {
                Self::analyze_expr_for_ref_mutation(
                    &while_loop.condition,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
                for stmt in &while_loop.body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::For(for_loop, _) => {
                match &for_loop.init {
                    ForInit::ForIn { iter, .. } => {
                        Self::analyze_expr_for_ref_mutation(
                            iter,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::analyze_statement_for_ref_mutation(
                            init,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                        Self::analyze_expr_for_ref_mutation(
                            condition,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                        Self::analyze_expr_for_ref_mutation(
                            update,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
                for stmt in &for_loop.body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        Self::analyze_statement_for_ref_mutation(
                            stmt,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
            }
            Statement::SetReturnExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceBodyExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceModuleExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::SetParamValue { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Return(None, _)
            | Statement::RemoveTarget(_)
            | Statement::SetParamType { .. }
            | Statement::SetReturnType { .. } => {}
        }
    }

    pub(super) fn ref_param_index_from_arg(
        arg: &shape_ast::ast::Expr,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
    ) -> Option<usize> {
        match arg {
            shape_ast::ast::Expr::Reference { expr: inner, .. } => match inner.as_ref() {
                shape_ast::ast::Expr::Identifier(name, _) => param_index_by_name
                    .get(name)
                    .copied()
                    .filter(|idx| caller_ref_params.get(*idx).copied().unwrap_or(false)),
                _ => None,
            },
            shape_ast::ast::Expr::Identifier(name, _) => param_index_by_name
                .get(name)
                .copied()
                .filter(|idx| caller_ref_params.get(*idx).copied().unwrap_or(false)),
            _ => None,
        }
    }
}


impl BytecodeCompiler {
    pub(super) fn analyze_expr_for_ref_mutation(
        expr: &shape_ast::ast::Expr,
        caller_name: &str,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
        callee_ref_params: &HashMap<String, Vec<bool>>,
        direct_mutates: &mut [bool],
        edges: &mut Vec<(String, usize, String, usize)>,
    ) {
        use shape_ast::ast::Expr;
        macro_rules! visit_expr {
            ($e:expr) => {
                Self::analyze_expr_for_ref_mutation(
                    $e,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                )
            };
        }
        macro_rules! visit_stmt {
            ($s:expr) => {
                Self::analyze_statement_for_ref_mutation(
                    $s,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                )
            };
        }

        match expr {
            Expr::Assign(assign, _) => {
                match assign.target.as_ref() {
                    Expr::Identifier(name, _) => {
                        if let Some(&idx) = param_index_by_name.get(name)
                            && caller_ref_params.get(idx).copied().unwrap_or(false)
                        {
                            direct_mutates[idx] = true;
                        }
                    }
                    Expr::IndexAccess { object, .. } | Expr::PropertyAccess { object, .. } => {
                        if let Expr::Identifier(name, _) = object.as_ref()
                            && let Some(&idx) = param_index_by_name.get(name)
                            && caller_ref_params.get(idx).copied().unwrap_or(false)
                        {
                            direct_mutates[idx] = true;
                        }
                    }
                    _ => {}
                }
                visit_expr!(&assign.value);
            }
            Expr::FunctionCall {
                name,
                args,
                named_args,
                ..
            } => {
                if let Some(callee_params) = callee_ref_params.get(name) {
                    for (arg_idx, arg) in args.iter().enumerate() {
                        if !callee_params.get(arg_idx).copied().unwrap_or(false) {
                            continue;
                        }
                        if let Some(caller_param_idx) = Self::ref_param_index_from_arg(
                            arg,
                            param_index_by_name,
                            caller_ref_params,
                        ) {
                            edges.push((
                                caller_name.to_string(),
                                caller_param_idx,
                                name.clone(),
                                arg_idx,
                            ));
                        }
                    }
                }
                // For callees not in the known function set (builtins, intrinsics,
                // imported functions), assume they do NOT mutate reference parameters.
                // Being too conservative here causes false B0004 errors when passing
                // non-identifier expressions (like object literals) to functions whose
                // parameters are inferred as references.

                for arg in args {
                    visit_expr!(arg);
                }

                for (_, arg) in named_args {
                    if let Some(idx) =
                        Self::ref_param_index_from_arg(arg, param_index_by_name, caller_ref_params)
                    {
                        direct_mutates[idx] = true;
                    }
                    visit_expr!(arg);
                }
            }
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                named_args,
                ..
            } => {
                let scoped_name = format!("{}::{}", namespace, function);
                if let Some(callee_params) = callee_ref_params.get(&scoped_name) {
                    for (arg_idx, arg) in args.iter().enumerate() {
                        if !callee_params.get(arg_idx).copied().unwrap_or(false) {
                            continue;
                        }
                        if let Some(caller_param_idx) = Self::ref_param_index_from_arg(
                            arg,
                            param_index_by_name,
                            caller_ref_params,
                        ) {
                            edges.push((
                                caller_name.to_string(),
                                caller_param_idx,
                                scoped_name.clone(),
                                arg_idx,
                            ));
                        }
                    }
                }

                for arg in args {
                    visit_expr!(arg);
                }

                for (_, arg) in named_args {
                    if let Some(idx) =
                        Self::ref_param_index_from_arg(arg, param_index_by_name, caller_ref_params)
                    {
                        direct_mutates[idx] = true;
                    }
                    visit_expr!(arg);
                }
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                visit_expr!(receiver);
                for arg in args {
                    visit_expr!(arg);
                }
                for (_, arg) in named_args {
                    visit_expr!(arg);
                }
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::TimeframeContext { expr: operand, .. }
            | Expr::UsingImpl { expr: operand, .. }
            | Expr::Reference { expr: operand, .. } => {
                visit_expr!(operand);
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                visit_expr!(left);
                visit_expr!(right);
            }
            Expr::PropertyAccess { object, .. } => {
                visit_expr!(object);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                visit_expr!(object);
                visit_expr!(index);
                if let Some(end) = end_index {
                    visit_expr!(end);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                visit_expr!(condition);
                visit_expr!(then_expr);
                if let Some(else_expr) = else_expr {
                    visit_expr!(else_expr);
                }
            }
            Expr::Array(items, _) => {
                for item in items {
                    visit_expr!(item);
                }
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for elem in row {
                        visit_expr!(elem);
                    }
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        shape_ast::ast::ObjectEntry::Field { value, .. } => {
                            visit_expr!(value);
                        }
                        shape_ast::ast::ObjectEntry::Spread(spread) => {
                            visit_expr!(spread);
                        }
                    }
                }
            }
            Expr::ListComprehension(comp, _) => {
                visit_expr!(&comp.element);
                for clause in &comp.clauses {
                    visit_expr!(&clause.iterable);
                    if let Some(filter) = &clause.filter {
                        visit_expr!(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                visit_expr!(value);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            if let Some(name) = assign.pattern.as_identifier()
                                && let Some(&idx) = param_index_by_name.get(name)
                                && caller_ref_params.get(idx).copied().unwrap_or(false)
                            {
                                direct_mutates[idx] = true;
                            }
                            visit_expr!(&assign.value);
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            visit_stmt!(stmt);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            visit_expr!(expr);
                        }
                    }
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    visit_stmt!(stmt);
                }
            }
            Expr::If(if_expr, _) => {
                visit_expr!(&if_expr.condition);
                visit_expr!(&if_expr.then_branch);
                if let Some(else_branch) = &if_expr.else_branch {
                    visit_expr!(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                visit_expr!(&while_expr.condition);
                visit_expr!(&while_expr.body);
            }
            Expr::For(for_expr, _) => {
                visit_expr!(&for_expr.iterable);
                visit_expr!(&for_expr.body);
            }
            Expr::Loop(loop_expr, _) => {
                visit_expr!(&loop_expr.body);
            }
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    visit_expr!(value);
                }
                visit_expr!(&let_expr.body);
            }
            Expr::Match(match_expr, _) => {
                visit_expr!(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        visit_expr!(guard);
                    }
                    visit_expr!(&arm.body);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    visit_expr!(&branch.expr);
                }
            }
            Expr::Annotated { target, .. } => {
                visit_expr!(target);
            }
            Expr::AsyncLet(async_let, _) => {
                visit_expr!(&async_let.expr);
            }
            Expr::AsyncScope(inner, _) => {
                visit_expr!(inner);
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    visit_stmt!(stmt);
                }
            }
            Expr::ComptimeFor(cf, _) => {
                visit_expr!(&cf.iterable);
                for stmt in &cf.body {
                    visit_stmt!(stmt);
                }
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    visit_expr!(value);
                }
            }
            Expr::WindowExpr(window_expr, _) => {
                match &window_expr.function {
                    shape_ast::ast::WindowFunction::Lag { expr, default, .. }
                    | shape_ast::ast::WindowFunction::Lead { expr, default, .. } => {
                        visit_expr!(expr);
                        if let Some(default) = default {
                            visit_expr!(default);
                        }
                    }
                    shape_ast::ast::WindowFunction::FirstValue(expr)
                    | shape_ast::ast::WindowFunction::LastValue(expr)
                    | shape_ast::ast::WindowFunction::NthValue(expr, _)
                    | shape_ast::ast::WindowFunction::Sum(expr)
                    | shape_ast::ast::WindowFunction::Avg(expr)
                    | shape_ast::ast::WindowFunction::Min(expr)
                    | shape_ast::ast::WindowFunction::Max(expr) => {
                        visit_expr!(expr);
                    }
                    shape_ast::ast::WindowFunction::Count(expr) => {
                        if let Some(expr) = expr {
                            visit_expr!(expr);
                        }
                    }
                    shape_ast::ast::WindowFunction::RowNumber
                    | shape_ast::ast::WindowFunction::Rank
                    | shape_ast::ast::WindowFunction::DenseRank
                    | shape_ast::ast::WindowFunction::Ntile(_) => {}
                }

                for partition_expr in &window_expr.over.partition_by {
                    visit_expr!(partition_expr);
                }
                if let Some(order_by) = &window_expr.over.order_by {
                    for (order_expr, _) in &order_by.columns {
                        visit_expr!(order_expr);
                    }
                }
            }
            Expr::FromQuery(fq, _) => {
                visit_expr!(&fq.source);
                for clause in &fq.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(expr) => {
                            visit_expr!(expr);
                        }
                        shape_ast::ast::QueryClause::OrderBy(items) => {
                            for item in items {
                                visit_expr!(&item.key);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            visit_expr!(element);
                            visit_expr!(key);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            visit_expr!(value);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            visit_expr!(source);
                            visit_expr!(left_key);
                            visit_expr!(right_key);
                        }
                    }
                }
                visit_expr!(&fq.select);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    visit_expr!(value);
                }
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        visit_expr!(value);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        visit_expr!(value);
                    }
                }
            },
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                visit_expr!(expr);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        visit_expr!(value);
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => {
                visit_expr!(expr);
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    visit_expr!(start);
                }
                if let Some(end) = end {
                    visit_expr!(end);
                }
            }
            Expr::DataRelativeAccess { reference, .. } => {
                visit_expr!(reference);
            }
            Expr::Break(Some(expr), _) | Expr::Return(Some(expr), _) => {
                visit_expr!(expr);
            }
            Expr::Literal(..)
            | Expr::Identifier(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::DateTime(..)
            | Expr::PatternRef(..)
            | Expr::Unit(..)
            | Expr::Duration(..)
            | Expr::Continue(..)
            | Expr::Break(None, _)
            | Expr::Return(None, _) => {}
        }
    }
}


impl BytecodeCompiler {
    pub(super) fn infer_reference_model(
        program: &Program,
    ) -> (
        HashMap<String, Vec<bool>>,
        HashMap<String, Vec<bool>>,
        HashMap<String, Vec<Option<String>>>,
    ) {
        let funcs = Self::collect_program_functions(program);
        let mut inference = shape_runtime::type_system::inference::TypeInferenceEngine::new();
        let (types, _) = inference.infer_program_best_effort(program);
        let inferred_ref_params = Self::infer_reference_params_from_types(program, &types);
        let inferred_param_type_hints = Self::infer_param_type_hints_from_types(program, &types);

        let mut effective_ref_params: HashMap<String, Vec<bool>> = HashMap::new();
        for (name, func) in &funcs {
            let inferred = inferred_ref_params.get(name).cloned().unwrap_or_default();
            let mut refs = vec![false; func.params.len()];
            for (idx, param) in func.params.iter().enumerate() {
                refs[idx] = param.is_reference || inferred.get(idx).copied().unwrap_or(false);
            }
            effective_ref_params.insert(name.clone(), refs);
        }

        let mut direct_mutates: HashMap<String, Vec<bool>> = HashMap::new();
        let mut edges: Vec<(String, usize, String, usize)> = Vec::new();

        for (name, func) in &funcs {
            let caller_refs = effective_ref_params
                .get(name)
                .cloned()
                .unwrap_or_else(|| vec![false; func.params.len()]);
            let mut direct = vec![false; func.params.len()];
            let mut param_index_by_name: HashMap<String, usize> = HashMap::new();
            for (idx, param) in func.params.iter().enumerate() {
                for param_name in param.get_identifiers() {
                    param_index_by_name.insert(param_name, idx);
                }
            }
            for stmt in &func.body {
                Self::analyze_statement_for_ref_mutation(
                    stmt,
                    name,
                    &param_index_by_name,
                    &caller_refs,
                    &effective_ref_params,
                    &mut direct,
                    &mut edges,
                );
            }
            direct_mutates.insert(name.clone(), direct);
        }

        let mut result = direct_mutates;
        let mut changed = true;
        while changed {
            changed = false;
            for (caller, caller_idx, callee, callee_idx) in &edges {
                let callee_mutates = result
                    .get(callee)
                    .and_then(|flags| flags.get(*callee_idx))
                    .copied()
                    .unwrap_or(false);
                if !callee_mutates {
                    continue;
                }
                if let Some(caller_flags) = result.get_mut(caller)
                    && let Some(flag) = caller_flags.get_mut(*caller_idx)
                    && !*flag
                {
                    *flag = true;
                    changed = true;
                }
            }
        }

        (inferred_ref_params, result, inferred_param_type_hints)
    }

    pub(super) fn inferred_type_to_hint_name(ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(annotation) => Some(annotation.to_type_string()),
            Type::Generic { base, args } => {
                let base_name = Self::inferred_type_to_hint_name(base)?;
                if args.is_empty() {
                    return Some(base_name);
                }
                let mut arg_names = Vec::with_capacity(args.len());
                for arg in args {
                    arg_names.push(Self::inferred_type_to_hint_name(arg)?);
                }
                Some(format!("{}<{}>", base_name, arg_names.join(", ")))
            }
            Type::Variable(_) | Type::Constrained { .. } | Type::Function { .. } => None,
        }
    }

    pub(super) fn infer_param_type_hints_from_types(
        program: &Program,
        inferred_types: &HashMap<String, Type>,
    ) -> HashMap<String, Vec<Option<String>>> {
        let funcs = Self::collect_program_functions(program);
        let mut hints = HashMap::new();

        for (name, func) in funcs {
            let mut param_hints = vec![None; func.params.len()];
            let Some(Type::Function { params, .. }) = inferred_types.get(&name) else {
                hints.insert(name, param_hints);
                continue;
            };

            for (idx, param) in func.params.iter().enumerate() {
                if param.type_annotation.is_some() || param.simple_name().is_none() {
                    continue;
                }
                if let Some(inferred_param_ty) = params.get(idx) {
                    param_hints[idx] = Self::inferred_type_to_hint_name(inferred_param_ty);
                }
            }

            hints.insert(name, param_hints);
        }

        hints
    }

    pub(crate) fn resolve_compiled_annotation_name(
        &self,
        annotation: &shape_ast::ast::Annotation,
    ) -> Option<String> {
        self.resolve_compiled_annotation_name_str(&annotation.name)
    }

    pub(crate) fn resolve_compiled_annotation_name_str(&self, name: &str) -> Option<String> {
        if self.program.compiled_annotations.contains_key(name) {
            return Some(name.to_string());
        }

        if name.contains("::") {
            return None;
        }

        for module_path in self.module_scope_stack.iter().rev() {
            let scoped = Self::qualify_module_symbol(module_path, name);
            if self.program.compiled_annotations.contains_key(&scoped) {
                return Some(scoped);
            }
        }

        if let Some(imported) = self.imported_annotations.get(name) {
            let hidden_name =
                Self::qualify_module_symbol(&imported.hidden_module_name, &imported.original_name);
            if self.program.compiled_annotations.contains_key(&hidden_name) {
                return Some(hidden_name);
            }
        }

        None
    }

    pub(crate) fn lookup_compiled_annotation(
        &self,
        annotation: &shape_ast::ast::Annotation,
    ) -> Option<(String, crate::bytecode::CompiledAnnotation)> {
        let resolved_name = self.resolve_compiled_annotation_name(annotation)?;
        let compiled = self.program.compiled_annotations.get(&resolved_name)?.clone();
        Some((resolved_name, compiled))
    }

    pub(crate) fn annotation_matches_compiled_name(
        &self,
        annotation: &shape_ast::ast::Annotation,
        compiled_name: &str,
    ) -> bool {
        self.resolve_compiled_annotation_name(annotation)
            .as_deref()
            == Some(compiled_name)
    }

    pub(crate) fn annotation_args_for_compiled_name(
        &self,
        annotations: &[shape_ast::ast::Annotation],
        compiled_name: &str,
    ) -> Vec<shape_ast::ast::Expr> {
        annotations
            .iter()
            .find(|annotation| self.annotation_matches_compiled_name(annotation, compiled_name))
            .map(|annotation| annotation.args.clone())
            .unwrap_or_default()
    }

    pub(crate) fn is_definition_annotation_target(
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> bool {
        matches!(
            target_kind,
            shape_ast::ast::functions::AnnotationTargetKind::Function
                | shape_ast::ast::functions::AnnotationTargetKind::Type
                | shape_ast::ast::functions::AnnotationTargetKind::Module
        )
    }

    /// Validate that an annotation is applicable to the requested target kind.
    pub(crate) fn validate_annotation_target_usage(
        &self,
        ann: &shape_ast::ast::Annotation,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        fallback_span: shape_ast::ast::Span,
    ) -> Result<()> {
        let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else {
            let span = if ann.span == shape_ast::ast::Span::DUMMY {
                fallback_span
            } else {
                ann.span
            };
            return Err(ShapeError::SemanticError {
                message: format!("Unknown annotation '@{}'", ann.name),
                location: Some(self.span_to_source_location(span)),
            });
        };

        let has_definition_lifecycle =
            compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some();
        if has_definition_lifecycle && !Self::is_definition_annotation_target(target_kind) {
            let target_label = format!("{:?}", target_kind).to_lowercase();
            let span = if ann.span == shape_ast::ast::Span::DUMMY {
                fallback_span
            } else {
                ann.span
            };
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Annotation '{}' defines definition-time lifecycle hooks (`on_define`/`metadata`) and cannot be applied to a {}. Allowed targets for these hooks are: function, type, module",
                    ann.name, target_label
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        if compiled.allowed_targets.is_empty() || compiled.allowed_targets.contains(&target_kind) {
            return Ok(());
        }

        let allowed: Vec<String> = compiled
            .allowed_targets
            .iter()
            .map(|k| format!("{:?}", k).to_lowercase())
            .collect();
        let target_label = format!("{:?}", target_kind).to_lowercase();

        let span = if ann.span == shape_ast::ast::Span::DUMMY {
            fallback_span
        } else {
            ann.span
        };

        Err(ShapeError::SemanticError {
            message: format!(
                "Annotation '{}' cannot be applied to a {}. Allowed targets: {}",
                ann.name,
                target_label,
                allowed.join(", ")
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Compile a program to bytecode
    pub fn compile(mut self, program: &Program) -> Result<BytecodeProgram> {
        // First: desugar the program (converts FromQuery to method chains, etc.)
        let mut program = program.clone();
        shape_ast::transform::desugar_program(&mut program);
        let analysis_program =
            shape_ast::transform::augment_program_with_generated_extends(&program);

        // Run the shared analyzer and surface diagnostics that are currently
        // proven reliable in the compiler execution path.
        let mut known_bindings: Vec<String> = self.module_bindings.keys().cloned().collect();
        let namespace_bindings = Self::collect_namespace_import_bindings(&analysis_program);
        // Inline: collect namespace and annotation import scope sources
        for item in &analysis_program.items {
            if let shape_ast::ast::Item::Import(import_stmt, _) = item {
                if import_stmt.from.is_empty() {
                    continue;
                }
                match &import_stmt.items {
                    shape_ast::ast::ImportItems::Namespace { name, alias } => {
                        let local_name = alias.clone().unwrap_or_else(|| name.clone());
                        self.module_scope_sources
                            .entry(local_name)
                            .or_insert_with(|| import_stmt.from.clone());
                    }
                    shape_ast::ast::ImportItems::Named(specs) => {
                        if specs.iter().any(|spec| spec.is_annotation) {
                            let hidden_module_name =
                                crate::module_resolution::hidden_annotation_import_module_name(
                                    &import_stmt.from,
                                );
                            self.module_scope_sources
                                .entry(hidden_module_name)
                                .or_insert_with(|| import_stmt.from.clone());
                        }
                    }
                }
            }
        }
        known_bindings.extend(namespace_bindings.iter().cloned());
        self.module_namespace_bindings
            .extend(namespace_bindings.into_iter());
        for namespace in self.module_namespace_bindings.clone() {
            let binding_idx = self.get_or_create_module_binding(&namespace);
            self.register_extension_module_schema(&namespace);
            let module_schema_name = format!("__mod_{}", namespace);
            if self
                .type_tracker
                .schema_registry()
                .get(&module_schema_name)
                .is_some()
            {
                self.set_module_binding_type_info(binding_idx, &module_schema_name);
            }
        }
        known_bindings.sort();
        known_bindings.dedup();
        let analysis_mode = if matches!(self.type_diagnostic_mode, TypeDiagnosticMode::RecoverAll) {
            TypeAnalysisMode::RecoverAll
        } else {
            TypeAnalysisMode::FailFast
        };
        if let Err(errors) = analyze_program_with_mode(
            &analysis_program,
            self.source_text.as_deref(),
            None,
            Some(&known_bindings),
            analysis_mode,
        ) {
            match self.type_diagnostic_mode {
                TypeDiagnosticMode::Strict => {
                    return Err(Self::type_errors_to_shape(errors));
                }
                TypeDiagnosticMode::ReliableOnly => {
                    let strict_errors: Vec<_> = errors
                        .into_iter()
                        .filter(|error| Self::should_emit_type_diagnostic(&error.error))
                        .collect();
                    if !strict_errors.is_empty() {
                        return Err(Self::type_errors_to_shape(strict_errors));
                    }
                }
                TypeDiagnosticMode::RecoverAll => {
                    self.errors.extend(
                        errors
                            .into_iter()
                            .map(Self::type_error_with_location_to_shape),
                    );
                }
            }
        }

        let (inferred_ref_params, inferred_ref_mutates, inferred_param_type_hints) =
            Self::infer_reference_model(&program);
        self.inferred_param_pass_modes =
            Self::build_param_pass_mode_map(&program, &inferred_ref_params, &inferred_ref_mutates);
        self.inferred_ref_params = inferred_ref_params;
        self.inferred_ref_mutates = inferred_ref_mutates;
        self.inferred_param_type_hints = inferred_param_type_hints;

        // Two-phase TypedObject field hoisting:
        //
        // Phase 1 (here, AST pre-pass): Collect all property assignments (e.g.,
        // `a.y = 2`) from the entire program BEFORE any function compilation.
        // This populates `hoisted_fields` so that `compile_typed_object_literal`
        // can allocate schema slots for future fields at object-creation time.
        // Without this pre-pass, the schema would be too small and a later
        // `a.y = 2` would require a schema migration at runtime.
        //
        // Phase 2 (per-function, MIR): During function compilation, MIR field
        // analysis (`mir::field_analysis::analyze_fields`) runs flow-sensitive
        // definite-initialization and liveness analysis. This detects:
        //   - `dead_fields`: fields that are written but never read (wasted slots)
        //   - `conditionally_initialized`: fields only assigned on some paths
        //
        // After MIR analysis, the compiler can cross-reference
        // `mir_field_analyses[func].dead_fields` to prune unused hoisted fields
        // from schemas. The dead_fields set uses `(SlotId, FieldIdx)` which must
        // be mapped to field names via the schema registry — see the integration
        // note in `compile_typed_object_literal`.
        {
            use shape_runtime::type_system::inference::PropertyAssignmentCollector;
            let assignments = PropertyAssignmentCollector::collect(&program);
            let grouped = PropertyAssignmentCollector::group_by_variable(&assignments);
            for (var_name, var_assignments) in grouped {
                let field_names: Vec<String> =
                    var_assignments.iter().map(|a| a.property.clone()).collect();
                self.hoisted_fields.insert(var_name, field_names);
            }
        }

        // First pass: collect all function definitions
        for item in &program.items {
            self.register_item_functions(item)?;
        }

        // MIR authority for non-function items: run borrow analysis on top-level
        // code before compilation. Errors in cleanly-lowered regions are emitted;
        // errors in fallback regions are suppressed (span-granular filtering).
        if let Err(e) = self.analyze_non_function_items_with_mir("__main__", &program.items) {
            self.errors.push(e);
        }

        // Start __main__ blob builder for top-level code.
        self.current_blob_builder = Some(FunctionBlobBuilder::new(
            "__main__".to_string(),
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        // Push a top-level drop scope so that block expressions and
        // statement-level VarDecls can track locals for auto-drop.
        self.push_drop_scope();
        self.non_function_mir_context_stack
            .push("__main__".to_string());

        // Second pass: compile all items (collect errors instead of early-returning)
        let item_count = program.items.len();
        for (idx, item) in program.items.iter().enumerate() {
            let is_last = idx == item_count - 1;
            let future_names =
                self.future_reference_use_names_for_remaining_items(&program.items[idx + 1..]);
            self.push_future_reference_use_names(future_names);
            let compile_result = self.compile_item_with_context(item, is_last);
            self.pop_future_reference_use_names();
            if let Err(e) = compile_result {
                self.errors.push(e);
            }
            self.release_unused_module_reference_borrows_for_remaining_items(
                &program.items[idx + 1..],
            );
        }
        self.non_function_mir_context_stack.pop();

        // Return collected errors before emitting Halt
        if !self.errors.is_empty() {
            if self.errors.len() == 1 {
                return Err(self.errors.remove(0));
            }
            return Err(shape_ast::error::ShapeError::MultiError(self.errors));
        }

        // Emit drops for top-level locals (from the top-level drop scope)
        self.pop_drop_scope()?;

        // Emit drops for top-level module bindings that have Drop impls
        {
            let bindings: Vec<(u16, bool)> = std::mem::take(&mut self.drop_module_bindings);
            for (binding_idx, is_async) in bindings.into_iter().rev() {
                self.emit_drop_call_for_module_binding(binding_idx, is_async);
            }
        }

        // Add halt instruction at the end
        self.emit(Instruction::simple(OpCode::Halt));

        // Store module_binding variable names for REPL persistence
        // Build a Vec<String> where index matches the module_binding variable index
        let mut module_binding_names = vec![String::new(); self.module_bindings.len()];
        for (name, &idx) in &self.module_bindings {
            module_binding_names[idx as usize] = name.clone();
        }
        self.program.module_binding_names = module_binding_names;

        // Store top-level locals count so executor can advance sp past them
        self.program.top_level_locals_count = self.next_local;

        // Persist storage hints for JIT width-aware lowering.
        self.populate_program_storage_hints();

        // Transfer type schema registry for TypedObject field resolution
        self.program.type_schema_registry = self.type_tracker.schema_registry().clone();

        // Transfer final function definitions after comptime mutation/specialization.
        self.program.expanded_function_defs = self.function_defs.clone();

        // Transfer monomorphization cache keys for diagnostics/testing.
        self.program.monomorphization_keys =
            self.monomorphization_cache.keys().cloned().collect();

        // Cache top-level MIR data for JIT v2 (MirToIR compilation of __main__).
        // The MIR and borrow analysis were computed by analyze_non_function_items_with_mir
        // above; we combine them with a storage plan here.
        {
            let mir_opt = self.mir_functions.get("__main__").cloned();
            let borrow_opt = self.mir_borrow_analyses.get("__main__").cloned();
            if let (Some(mut mir), Some(borrow_analysis)) = (mir_opt, borrow_opt) {
                if !self.closure_function_ids.is_empty() {
                    let mut closure_idx = 0;
                    let closure_ids = self.closure_function_ids.clone();
                    let mut has_capture = false;
                    for block in &mut mir.blocks {
                        for stmt in &mut block.statements {
                            let is_placeholder = matches!(&stmt.kind, crate::mir::types::StatementKind::Assign(_, crate::mir::types::Rvalue::Use(crate::mir::types::Operand::Constant(crate::mir::types::MirConstant::ClosurePlaceholder))));
                            if is_placeholder {
                                if has_capture { stmt.kind = crate::mir::types::StatementKind::Nop; has_capture = false; }
                                else if closure_idx < closure_ids.len() {
                                    let (ref name, _) = closure_ids[closure_idx];
                                    let slot = match &stmt.kind { crate::mir::types::StatementKind::Assign(p, _) => p.root_local(), _ => unreachable!() };
                                    stmt.kind = crate::mir::types::StatementKind::Assign(crate::mir::types::Place::Local(slot), crate::mir::types::Rvalue::Use(crate::mir::types::Operand::Constant(crate::mir::types::MirConstant::Function(name.clone()))));
                                    closure_idx += 1;
                                }
                                continue;
                            }
                            if let crate::mir::types::StatementKind::ClosureCapture { function_id, .. } = &mut stmt.kind {
                                if closure_idx < closure_ids.len() { let (_, idx) = closure_ids[closure_idx]; *function_id = Some(idx); closure_idx += 1; has_capture = true; }
                            }
                        }
                    }
                }
                use std::collections::{HashMap as StdHashMap, HashSet as StdHashSet};
                let planner_input = crate::mir::storage_planning::StoragePlannerInput {
                    mir: &mir,
                    analysis: &borrow_analysis,
                    binding_semantics: &StdHashMap::new(),
                    closure_captures: &StdHashSet::new(),
                    mutable_captures: &StdHashSet::new(),
                    had_fallbacks: true, // conservative: top-level MIR often has fallbacks
                };
                let storage_plan = crate::mir::storage_planning::plan_storage(&planner_input);
                self.program.top_level_mir =
                    Some(std::sync::Arc::new(crate::bytecode::MirFunctionData {
                        mir,
                        storage_plan,
                        borrow_analysis,
                    }));
            }
        }

        // Finalize the __main__ blob and build the content-addressed program.
        self.build_content_addressed_program();

        // Transfer content-addressed program to the bytecode output.
        self.program.content_addressed = self.content_addressed_program.take();
        if self.program.functions.is_empty() {
            self.program.function_blob_hashes.clear();
        } else {
            if self.function_hashes_by_id.len() < self.program.functions.len() {
                self.function_hashes_by_id
                    .resize(self.program.functions.len(), None);
            } else if self.function_hashes_by_id.len() > self.program.functions.len() {
                self.function_hashes_by_id
                    .truncate(self.program.functions.len());
            }
            self.program.function_blob_hashes = self.function_hashes_by_id.clone();
        }

        // Transfer source text for error messages
        if let Some(source) = self.source_text {
            // Set in legacy field for backward compatibility
            self.program.debug_info.source_text = source.clone();
            // Also set in source map if not already set
            if self.program.debug_info.source_map.files.is_empty() {
                self.program
                    .debug_info
                    .source_map
                    .add_file("<main>".to_string());
            }
            if self.program.debug_info.source_map.source_texts.is_empty() {
                self.program
                    .debug_info
                    .source_map
                    .set_source_text(0, source);
            }
        }

        Ok(self.program)
    }

    /// Compile a program to bytecode with source text for error messages
    pub fn compile_with_source(
        mut self,
        program: &Program,
        source: &str,
    ) -> Result<BytecodeProgram> {
        self.set_source(source);
        self.compile(program)
    }

    /// Compile a program using the module graph for import resolution.
    ///
    /// This is the graph-driven compilation pipeline. Modules compile in
    /// topological order using the graph for cross-module name resolution.
    /// No AST inlining occurs — each module's imports are resolved from
    /// the graph's `ResolvedImport` entries.
    pub fn compile_with_graph(
        self,
        root_program: &Program,
        graph: std::sync::Arc<crate::module_graph::ModuleGraph>,
    ) -> Result<BytecodeProgram> {
        self.compile_with_graph_and_prelude(root_program, graph, &[])
    }

    /// Compile with graph and prelude information.
    ///
    /// All modules (including prelude dependencies) compile uniformly
    /// through the normal module path. The `prelude_paths` parameter is
    /// retained for API compatibility but no longer used.
    pub fn compile_with_graph_and_prelude(
        mut self,
        root_program: &Program,
        graph: std::sync::Arc<crate::module_graph::ModuleGraph>,
        _prelude_paths: &[String],
    ) -> Result<BytecodeProgram> {
        use crate::module_graph::ModuleSourceKind;

        self.module_graph = Some(graph.clone());

        // Phase 1: Compile dependency modules in topological order.
        for &dep_id in graph.topo_order() {
            let dep_node = graph.node(dep_id);
            match dep_node.source_kind {
                ModuleSourceKind::NativeModule => {
                    self.register_graph_imports_for_module(dep_id, &graph)?;
                }
                ModuleSourceKind::ShapeSource | ModuleSourceKind::Hybrid => {
                    self.compile_module_from_graph(dep_id, &graph)?;
                }
                ModuleSourceKind::CompiledBytecode => {
                    // Should have been rejected during graph construction.
                    return Err(shape_ast::error::ShapeError::ModuleError {
                        message: format!(
                            "Module '{}' is only available as pre-compiled bytecode",
                            dep_node.canonical_path
                        ),
                        module_path: None,
                    });
                }
            }
        }

        // Phase 2: Compile the root module using the graph for its imports.
        // Register root's imports from the graph
        self.register_graph_imports_for_module(graph.root_id(), &graph)?;

        // Strip import items from root program (imports already resolved via graph)
        let mut stripped_program = root_program.clone();
        stripped_program.items.retain(|item| !matches!(item, shape_ast::ast::Item::Import(..)));

        // Compile the stripped root program using the standard two-pass pipeline
        self.compile(&stripped_program)
    }

    /// Compile a single module from the graph.
    ///
    /// All modules (including prelude dependencies) compile uniformly:
    /// pushes the module scope, qualifies items, registers all symbol kinds,
    /// compiles bodies, creates module binding object.
    fn compile_module_from_graph(
        &mut self,
        module_id: crate::module_graph::ModuleId,
        graph: &crate::module_graph::ModuleGraph,
    ) -> Result<()> {
        let node = graph.node(module_id);
        let ast = match &node.ast {
            Some(ast) => ast.clone(),
            None => return Ok(()), // NativeModule / CompiledBytecode
        };

        let module_path = node.canonical_path.clone();

        // All modules compile uniformly through the normal module path.
        // Set allow_internal_builtins for stdlib modules.
        let prev_allow = self.allow_internal_builtins;
        if module_path.starts_with("std::") {
            self.allow_internal_builtins = true;
        }

        self.module_scope_stack.push(module_path.clone());

        // 1. Register this module's imports from the graph
        self.register_graph_imports_for_module(module_id, graph)?;

        // 2. Filter out import statements, qualify remaining items
        let mut qualified_items = Vec::new();
        for item in &ast.items {
            if matches!(item, shape_ast::ast::Item::Import(..)) {
                continue;
            }
            qualified_items.push(self.qualify_module_item(item, &module_path)?);
        }

        // 3. Phase 1: Register functions in global table with qualified names
        for item in &qualified_items {
            self.register_missing_module_items(item)?;
        }

        // 4. Phase 2: Compile function bodies
        self.non_function_mir_context_stack
            .push(module_path.clone());
        let compile_result = (|| -> Result<()> {
            for (idx, qualified) in qualified_items.iter().enumerate() {
                let future_names = self
                    .future_reference_use_names_for_remaining_items(&qualified_items[idx + 1..]);
                self.push_future_reference_use_names(future_names);
                let result = self.compile_item_with_context(qualified, false);
                self.pop_future_reference_use_names();
                result?;
                self.release_unused_module_reference_borrows_for_remaining_items(
                    &qualified_items[idx + 1..],
                );
            }
            Ok(())
        })();
        self.non_function_mir_context_stack.pop();
        compile_result?;

        // 5. Build module object and store in canonical binding
        let exports = self.collect_module_runtime_exports(
            &ast.items
                .iter()
                .filter(|i| !matches!(i, shape_ast::ast::Item::Import(..)))
                .cloned()
                .collect::<Vec<_>>(),
            &module_path,
        );
        let span = shape_ast::ast::Span::default();
        let entries: Vec<shape_ast::ast::ObjectEntry> = exports
            .into_iter()
            .map(|(name, value_ident)| shape_ast::ast::ObjectEntry::Field {
                key: name,
                value: shape_ast::ast::Expr::Identifier(value_ident, span),
                type_annotation: None,
            })
            .collect();
        let module_object = shape_ast::ast::Expr::Object(entries, span);
        self.compile_expr(&module_object)?;

        let binding_idx = self.get_or_create_module_binding(&module_path);
        self.emit(Instruction::new(
            OpCode::StoreModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        self.propagate_initializer_type_to_slot(binding_idx, false, false);

        self.module_scope_stack.pop();
        self.allow_internal_builtins = prev_allow;
        Ok(())
    }

    /// Compile an imported module's AST to a standalone BytecodeProgram.
    ///
    /// This takes the Module's AST (Program), compiles all exported functions
    /// to bytecode, and returns the compiled program along with a mapping of
    /// exported function names to their function indices in the compiled output.
    ///
    /// The returned `BytecodeProgram` and function name mapping allow the import
    /// handler to resolve imported function calls to the correct bytecode indices.
    ///
    /// Currently handles function exports only. Types and values can be added later.
    pub fn compile_module_ast(
        module_ast: &Program,
    ) -> Result<(BytecodeProgram, HashMap<String, usize>)> {
        let mut compiler = BytecodeCompiler::new();
        // Stdlib modules need access to __* builtins (intrinsics, into, etc.)
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(module_ast)?;

        // Build name → function index mapping for exported functions
        let mut export_map = HashMap::new();
        for (idx, func) in bytecode.functions.iter().enumerate() {
            export_map.insert(func.name.clone(), idx);
        }

        Ok((bytecode, export_map))
    }
}
