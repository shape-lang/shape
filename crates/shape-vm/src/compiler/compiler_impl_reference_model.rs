use super::*;

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
        HashMap<String, String>,
    ) {
        let funcs = Self::collect_program_functions(program);
        let mut inference = shape_runtime::type_system::inference::TypeInferenceEngine::new();
        let (types, _) = inference.infer_program_best_effort(program);
        let inferred_ref_params = Self::infer_reference_params_from_types(program, &types);
        let inferred_param_type_hints = Self::infer_param_type_hints_from_types(program, &types);
        let inferred_return_type_hints = Self::infer_return_type_hints_from_types(program, &types);

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

        (inferred_ref_params, result, inferred_param_type_hints, inferred_return_type_hints)
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

    /// Phase 3e: extract a hint name for each function's inferred return
    /// type. Used to populate `type_tracker.function_return_types` so call
    /// expressions can recover numeric types (and string/bool primitives
    /// via `set_function_return_type`) when the source has no explicit
    /// return-type annotation.
    pub(super) fn infer_return_type_hints_from_types(
        program: &Program,
        inferred_types: &HashMap<String, Type>,
    ) -> HashMap<String, String> {
        let funcs = Self::collect_program_functions(program);
        let mut hints = HashMap::new();
        for (name, _) in funcs {
            let Some(Type::Function { returns, .. }) = inferred_types.get(&name) else {
                continue;
            };
            if let Some(rt_name) = Self::inferred_type_to_hint_name(returns) {
                hints.insert(name, rt_name);
            }
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

        // W9: handle qualified `@local::name` form by resolving the local
        // namespace prefix to its canonical module path, then looking up
        // `canonical::name` in compiled_annotations.
        if let Some((local_prefix, rest)) = name.split_once("::") {
            // First try graph-driven namespace map (canonical for graph compile).
            if let Some(canonical) = self.graph_namespace_map.get(local_prefix) {
                let qualified = Self::qualify_module_symbol(canonical, rest);
                if self.program.compiled_annotations.contains_key(&qualified) {
                    return Some(qualified);
                }
            }
            // Fall back to module_scope_sources (legacy / non-graph compile).
            if let Some(canonical) = self.module_scope_sources.get(local_prefix) {
                let qualified = Self::qualify_module_symbol(canonical, rest);
                if self.program.compiled_annotations.contains_key(&qualified) {
                    return Some(qualified);
                }
            }
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
        let compiled = self
            .program
            .compiled_annotations
            .get(&resolved_name)?
            .clone();
        Some((resolved_name, compiled))
    }

    pub(crate) fn annotation_matches_compiled_name(
        &self,
        annotation: &shape_ast::ast::Annotation,
        compiled_name: &str,
    ) -> bool {
        self.resolve_compiled_annotation_name(annotation).as_deref() == Some(compiled_name)
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
                        // W9: register annotation-import scope source against
                        // the canonical module path. The synthetic hidden-module
                        // name is no longer used; use-site annotation resolution
                        // looks up `canonical_path::name` directly.
                        if specs.iter().any(|spec| spec.is_annotation) {
                            self.module_scope_sources
                                .entry(import_stmt.from.clone())
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

        let (
            inferred_ref_params,
            inferred_ref_mutates,
            inferred_param_type_hints,
            inferred_return_type_hints,
        ) = Self::infer_reference_model(&program);
        self.inferred_param_pass_modes =
            Self::build_param_pass_mode_map(&program, &inferred_ref_params, &inferred_ref_mutates);
        self.inferred_ref_params = inferred_ref_params;
        self.inferred_ref_mutates = inferred_ref_mutates;
        self.inferred_param_type_hints = inferred_param_type_hints;
        // Phase 3e: register inferred return types so function-call
        // compilation can recover the numeric type even for sources with
        // no explicit `-> T` annotation.
        for (fn_name, ret_ty) in &inferred_return_type_hints {
            self.type_tracker
                .register_function_return_type(fn_name, ret_ty);
        }

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
            use shape_ast::ast::{Expr, Literal};
            use shape_runtime::type_schema::FieldType;
            let assignments = PropertyAssignmentCollector::collect(&program);
            let grouped = PropertyAssignmentCollector::group_by_variable(&assignments);
            // Phase 3e: infer a primitive FieldType for each hoisted field
            // when the RHS is a literal whose type is statically known.
            // Falls back to FieldType::Any (the prior behavior) for
            // non-literal RHS or types we can't map.
            let infer_lit = |expr: &Expr| -> Option<FieldType> {
                match expr {
                    Expr::Literal(Literal::Int(_), _) => Some(FieldType::I64),
                    Expr::Literal(Literal::Number(_), _) => Some(FieldType::F64),
                    Expr::Literal(Literal::Decimal(_), _) => Some(FieldType::Decimal),
                    Expr::Literal(Literal::Bool(_), _) => Some(FieldType::Bool),
                    Expr::Literal(Literal::String(_), _) => Some(FieldType::String),
                    _ => None,
                }
            };
            for (var_name, var_assignments) in grouped {
                let field_names: Vec<String> =
                    var_assignments.iter().map(|a| a.property.clone()).collect();
                let mut type_map: std::collections::HashMap<String, FieldType> =
                    std::collections::HashMap::new();
                for a in &var_assignments {
                    if let Some(ft) = infer_lit(&a.value_expr) {
                        type_map.insert(a.property.clone(), ft);
                    }
                }
                if !type_map.is_empty() {
                    self.hoisted_field_types.insert(var_name.clone(), type_map);
                }
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

        // Register root's imports from the module graph. This emits alias
        // copy instructions (e.g. `set = std::core::set`) and MUST happen
        // INSIDE the `__main__` blob — emitting before the blob started
        // would leave the copies in an unreachable gap, so at runtime the
        // alias binding would remain None and `set::contains(...)` would
        // read a None callable and raise `InvalidCall`.
        if let Some(graph) = self.module_graph.clone() {
            let root_id = graph.root_id();
            self.register_graph_imports_for_module(root_id, &graph)?;
        }

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
            // E+5.5 Unit C step 2: capture the final expression's return-kind
            // signal RIGHT AFTER the last item compiles, before drop-scope
            // emission and Halt overwrite `last_expr_*`. The captured value
            // is consumed in `populate_program_storage_hints` to populate
            // `top_level_frame.return_kind` for the host-boundary
            // ValueWord synthesis.
            if is_last && self.errors.is_empty() {
                // Per ADR-006 §2.7.5.1, `infer_top_level_return_kind` /
                // `infer_top_level_return_kind_from_item` carry "kind not
                // yet proven" as `Option::None` — `.or_else(...)` falls
                // back to the AST-driven path when the state-driven one
                // produced no kind.
                let kind = self
                    .infer_top_level_return_kind()
                    .or_else(|| self.infer_top_level_return_kind_from_item(item));
                self.top_level_program_return_kind = kind;
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
        self.program.monomorphization_keys = self.monomorphization_cache.keys().cloned().collect();

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
                            let is_placeholder = matches!(
                                &stmt.kind,
                                crate::mir::types::StatementKind::Assign(
                                    _,
                                    crate::mir::types::Rvalue::Use(
                                        crate::mir::types::Operand::Constant(
                                            crate::mir::types::MirConstant::ClosurePlaceholder
                                        )
                                    )
                                )
                            );
                            if is_placeholder {
                                if has_capture {
                                    stmt.kind = crate::mir::types::StatementKind::Nop;
                                    has_capture = false;
                                } else if closure_idx < closure_ids.len() {
                                    let (ref name, _) = closure_ids[closure_idx];
                                    let slot = match &stmt.kind {
                                        crate::mir::types::StatementKind::Assign(p, _) => {
                                            p.root_local()
                                        }
                                        _ => unreachable!(),
                                    };
                                    stmt.kind = crate::mir::types::StatementKind::Assign(
                                        crate::mir::types::Place::Local(slot),
                                        crate::mir::types::Rvalue::Use(
                                            crate::mir::types::Operand::Constant(
                                                crate::mir::types::MirConstant::Function(
                                                    name.clone(),
                                                ),
                                            ),
                                        ),
                                    );
                                    closure_idx += 1;
                                }
                                continue;
                            }
                            if let crate::mir::types::StatementKind::ClosureCapture {
                                function_id,
                                ..
                            } = &mut stmt.kind
                            {
                                if closure_idx < closure_ids.len() {
                                    let (_, idx) = closure_ids[closure_idx];
                                    *function_id = Some(idx);
                                    closure_idx += 1;
                                    has_capture = true;
                                }
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
                    callee_summaries: Some(&self.function_borrow_summaries),
                };
                let storage_plan = crate::mir::storage_planning::plan_storage(&planner_input);

                // ADR-006 §2.7.5 stamp-at-compile-time, Phase 3
                // cluster-0 Round 16 W17-narrow-follow-up-A: thread
                // schema ids on top-level MIR `ObjectStore`
                // statements (canonical Smoke 3 site — `let t = X {}`
                // is top-level). Same back-patch as the per-function
                // path at `compiler/functions.rs` post-closure-id
                // patching; reads `mir.local_struct_type_names` +
                // `type_tracker.schema_registry()` to align with the
                // parallel bytecode-side `OpCode::NewTypedObject`
                // operand.
                crate::compiler::mir_schema_threading::back_patch_schema_ids(
                    &mut mir,
                    &mut self.type_tracker,
                );

                self.program.top_level_mir =
                    Some(std::sync::Arc::new(crate::bytecode::MirFunctionData {
                        mir,
                        storage_plan,
                        borrow_analysis,
                    }));
            }
        }

        // ADR-006 §2.7.5 conduit: stamp per-MIR-slot `ConcreteType` for
        // top-level code by walking the cached top-level MIR. The JIT
        // MirToIR reads this side-table (`BytecodeProgram.
        // top_level_local_concrete_types`) to drive the v2 typed-array
        // fast path (avoiding `Rvalue::Aggregate` surface-and-stop) and
        // the TypedObject `ObjectStore` short-circuit.
        //
        // Why MIR-walk rather than bytecode-compiler slot mapping: top-
        // level code allocates the user's bindings as module_bindings
        // (NOT bytecode locals — `self.next_local` is 0 at top level),
        // so the bytecode-compiler's per-local side-tables do not
        // carry top-level `let p = Point{...}` slots. The cached top-
        // level MIR already encodes the structural type information
        // through `StatementKind::{ObjectStore, ArrayStore, EnumStore}`
        // — the MIR-level kind-source statements emitted for
        // struct/enum/array construction. The walk is purely from the
        // proven MIR shape; no runtime decode, no Bool-default fallback.
        //
        // The result is indexed by MIR `SlotId` (matching MirToIR's
        // `concrete_type_for_slot` / `is_v2_typed_array_slot` indexing
        // exactly). `ConcreteType::Void` per slot means "no
        // information inferred" — a real enum variant per §2.7.5.1, not
        // a Bool-default fallback per forbidden #9.
        //
        // The top-level conduit walk is deferred a few lines down — it
        // runs AFTER the per-function return-type side-table is built,
        // so the Call-terminator destination stamping in the walk has
        // access to callee return types via the resolver. See the
        // W12-jit-call-return-kind block below.

        // ADR-006 §2.7.5 conduit (W12-jit-call-return-kind close, 2026-05-12):
        // Per-user-function declared return ConcreteType, built first so the
        // per-function and top-level conduit passes can consume it via the
        // callee-return resolver. Returns are classified from the AST
        // `FunctionDef.return_type` (preserved via `expanded_function_defs`)
        // through `concrete_type_from_annotation` (already used for HashMap
        // key/value extraction). When the function has no annotation or the
        // annotation doesn't reduce to a known shape, the entry stays
        // `ConcreteType::Void` per §2.7.5.1 — NOT a Bool-default fallback.
        let mut per_fn_ret: Vec<shape_value::v2::ConcreteType> =
            Vec::with_capacity(self.program.functions.len());
        for func in &self.program.functions {
            let ct = self
                .program
                .expanded_function_defs
                .get(&func.name)
                .and_then(|fd| fd.return_type.as_ref())
                .and_then(|ann| {
                    crate::compiler::v2_map_emission::concrete_type_from_annotation(
                        ann,
                    )
                })
                .unwrap_or(shape_value::v2::ConcreteType::Void);
            per_fn_ret.push(ct);
        }
        self.program.function_return_concrete_types = per_fn_ret;

        // Build the callee-return resolver: maps `MirConstant::Function(name)`
        // to the callee's declared return ConcreteType via the side-table
        // just populated. Used by the conduit passes below to stamp
        // `TerminatorKind::Call` destination slots. `None` for unknown /
        // unannotated / void-returning functions — the destination slot
        // stays `Void` (no fabrication).
        let name_to_idx: std::collections::HashMap<String, usize> = self
            .program
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name.clone(), i))
            .collect();
        let returns_vec = self.program.function_return_concrete_types.clone();
        let callee_returns = |name: &str| -> Option<shape_value::v2::ConcreteType> {
            let idx = *name_to_idx.get(name)?;
            let ct = returns_vec.get(idx)?;
            if matches!(ct, shape_value::v2::ConcreteType::Void) {
                None
            } else {
                Some(ct.clone())
            }
        };

        // ADR-006 §2.7.5 — Phase 3 cluster-0 Round 13 T1' commit 2:
        // method-returns resolver for trait-method dispatch return-kind
        // classification. Chains:
        //   `find_default_trait_impl_for_type_method(type_name, method_name)
        //    → trait impl function name (e.g. "X::name")
        //    → function_return_concrete_types[function_index]
        //    → declared return ConcreteType (e.g. ConcreteType::String)`
        //
        // Used by the conduit producer to stamp `TerminatorKind::Call`
        // destination slots for `MirConstant::Method(_)` arms with a
        // receiver slot whose struct type name was recorded in MIR
        // (`mir.local_struct_type_names`, T1' gap 1 closure). `None` at
        // any link in the chain means "no information" — the destination
        // slot stays `Void` per §2.7.5.1 (no fabricated default).
        //
        // Gap 3 closure (commit 1, `desugar_impl_method` trait
        // declaration return-type substitution) ensures
        // `function_return_concrete_types["X::name"]` carries the trait's
        // declared `ConcreteType::String` even when the impl source
        // doesn't repeat the `: string` annotation.
        let trait_method_symbols = self.program.trait_method_symbols.clone();
        let find_trait_impl_default_suffix =
            |type_name: &str, method_name: &str| -> Option<String> {
                // Mirror BytecodeProgram::find_default_trait_impl_for_type_method
                // semantics (the canonical helper at
                // `crates/shape-vm/src/bytecode/program_impl.rs:151`)
                // without borrowing `self.program` — the closure must be
                // passable by reference to the conduit producer
                // alongside `callee_returns`. The "__default__" selector
                // string is `DEFAULT_TRAIT_IMPL_SELECTOR` at
                // `crates/shape-vm/src/bytecode.rs:15`; inlined here to
                // avoid the borrow.
                let default_suffix = format!(
                    "::{}::__default__::{}",
                    type_name, method_name
                );
                for (key, func_name) in &trait_method_symbols {
                    if key.ends_with(&default_suffix) {
                        return Some(func_name.clone());
                    }
                }
                let type_segment = format!("::{}::", type_name);
                let suffix = format!("::{}", method_name);
                let mut matches: Vec<String> = Vec::new();
                for (key, func_name) in &trait_method_symbols {
                    if key.contains(&type_segment) && key.ends_with(&suffix) {
                        matches.push(func_name.clone());
                    }
                }
                // Multi-trait method-name disambiguation (audit §5):
                // when multiple traits declare `method()` for the same
                // receiver type, we cannot determine the return
                // ConcreteType uniquely from name alone — return None so
                // the downstream classifier surfaces unstamped.
                if matches.len() == 1 {
                    Some(matches.pop().unwrap())
                } else {
                    None
                }
            };
        let method_returns =
            |type_name: &str, method_name: &str| -> Option<shape_value::v2::ConcreteType> {
                let func_name = find_trait_impl_default_suffix(type_name, method_name)?;
                let idx = *name_to_idx.get(&func_name)?;
                let ct = returns_vec.get(idx)?;
                if matches!(ct, shape_value::v2::ConcreteType::Void) {
                    None
                } else {
                    Some(ct.clone())
                }
            };

        // ADR-006 §2.7.5 V3-S6b conduit consumer: monomorph-method
        // resolver. Reads `BytecodeProgram.monomorphized_method_call_sites`
        // populated by `try_monomorphize_method_call` /
        // `_with_closures` at bytecode-compile time, then chains the
        // looked-up specialized FunctionId through `returns_vec` (the
        // local clone of `function_return_concrete_types`) to recover the
        // callee specialization's declared return type. The closure
        // closes over the `current_function` half of the composite key
        // — top-level uses `None`; per-fn loop below uses
        // `Some(fn_idx)`.
        let monomorph_call_sites =
            self.program.monomorphized_method_call_sites.clone();
        let monomorph_method_returns_top = |span: shape_ast::ast::span::Span|
            -> Option<shape_value::v2::ConcreteType>
        {
            let idx = *monomorph_call_sites.get(&(span, None))?;
            let ct = returns_vec.get(idx)?;
            if matches!(ct, shape_value::v2::ConcreteType::Void) {
                None
            } else {
                Some(ct.clone())
            }
        };

        // cluster-2-cw-IB-class-b (2026-05-16, supervisor R3 binding-
        // ratified): value-call return-ConcreteType resolver. Consumes
        // the side-table populated at `compile_expr_function_call`'s
        // value-call branch and returns the inferred ConcreteType
        // result for closure-bound calls. Top-level conduit closes
        // over `None` for the caller half of the composite key — same
        // convention as `monomorph_method_returns_top`.
        let value_call_sites =
            self.program.value_call_return_concrete_types.clone();
        let value_call_returns_top = |span: shape_ast::ast::span::Span|
            -> Option<shape_value::v2::ConcreteType>
        {
            let ct = value_call_sites.get(&(span, None))?.clone();
            if matches!(ct, shape_value::v2::ConcreteType::Void) {
                None
            } else {
                Some(ct)
            }
        };

        // Re-run top-level conduit with the callee-return resolver so the
        // `let r = divide(10, 2)` slot picks up `Result(I64, String)` from
        // the Call terminator. (The first run above stamped `Void` for
        // Call destinations since no resolver was available.) The
        // method-returns resolver is also threaded so `t.name()`-style
        // trait-method dispatch destinations pick up the trait's declared
        // return ConcreteType. The V3-S6b monomorph-method resolver is
        // threaded so `arr.map(...).sum()` chains have the `.map()`
        // destination stamped with the specialized callee's return
        // ConcreteType.
        if let Some(ref mir_data) = self.program.top_level_mir {
            let concrete_types =
                crate::compiler::helpers::infer_top_level_concrete_types_from_mir_with_resolvers(
                    &mir_data.mir,
                    Some(&callee_returns),
                    Some(&method_returns),
                    Some(&monomorph_method_returns_top),
                    Some(&value_call_returns_top),
                );
            self.program.top_level_local_concrete_types = concrete_types;
        }

        // ADR-006 §2.7.5 conduit (W12-jit-aggregate-non-array close,
        // 2026-05-12): same MIR-walk inference applied per user function.
        // The producer (`infer_top_level_concrete_types_from_mir`) is
        // generic over any MirFunction — its name is historical from the
        // earlier top-level-only landing (Round 3). User-function bodies
        // hit the JIT consumer at
        // `crates/shape-jit/src/compiler/program.rs::compile_function_with_user_funcs`,
        // which currently passes `concrete_types: Vec::new()` and therefore
        // surfaces `Rvalue::Aggregate` for every `Ok(v)` / `Err(e)` /
        // `Some(x)` / struct-literal construction inside a user function
        // body (Smoke 1.5 `divide`, Smoke 2 `first_positive`, 28 stdlib
        // helpers verified at audit time).
        //
        // The callee-return resolver is also threaded here so user-function
        // bodies that call other user functions (e.g. `divide` calls a
        // helper) propagate the helper's return ConcreteType into their
        // own slot, recursing through the conduit.
        //
        // `ConcreteType::Void` per slot per §2.7.5.1 — NOT a Bool-default
        // fallback per forbidden #9. Functions without `mir_data` get an
        // empty inner vec; downstream consumers fall back to the legacy
        // NaN-boxed path naturally.
        let mut per_fn: Vec<Vec<shape_value::v2::ConcreteType>> =
            Vec::with_capacity(self.program.functions.len());
        for (fn_idx, func) in self.program.functions.iter().enumerate() {
            if let Some(ref mir_data) = func.mir_data {
                // ADR-006 §2.7.5 V3-S6b conduit consumer: per-fn variant
                // of the monomorph-method resolver. Closes over the
                // calling function's index for the composite-key lookup
                // — must match the value `try_monomorphize_method_call`
                // recorded in `self.current_function` at populate time
                // (i.e. `Some(fn_idx)` here matches the populator's
                // post-monomorphization specialized caller FunctionId).
                let current_fn = Some(fn_idx);
                let monomorph_method_returns_per_fn = |span: shape_ast::ast::span::Span|
                    -> Option<shape_value::v2::ConcreteType>
                {
                    let idx = *monomorph_call_sites.get(&(span, current_fn))?;
                    let ct = returns_vec.get(idx)?;
                    if matches!(ct, shape_value::v2::ConcreteType::Void) {
                        None
                    } else {
                        Some(ct.clone())
                    }
                };
                // cluster-2-cw-IB-class-b: per-fn variant of the value-call
                // return-ConcreteType resolver. Same composite-key
                // discipline as monomorph_method_returns_per_fn above —
                // closes over `Some(fn_idx)` so calls inside user-function
                // bodies pick up their own caller-context entries.
                let value_call_returns_per_fn = |span: shape_ast::ast::span::Span|
                    -> Option<shape_value::v2::ConcreteType>
                {
                    let ct = value_call_sites.get(&(span, current_fn))?.clone();
                    if matches!(ct, shape_value::v2::ConcreteType::Void) {
                        None
                    } else {
                        Some(ct)
                    }
                };
                per_fn.push(
                    crate::compiler::helpers::infer_top_level_concrete_types_from_mir_with_resolvers(
                        &mir_data.mir,
                        Some(&callee_returns),
                        Some(&method_returns),
                        Some(&monomorph_method_returns_per_fn),
                        Some(&value_call_returns_per_fn),
                    ),
                );
            } else {
                per_fn.push(Vec::new());
            }
        }
        self.program.function_local_concrete_types = per_fn;

        // Closure-spec Phase H1: build a `function_id → ClosureLayout` side
        // table for the JIT worker. `emit_heap_closure` consumes this to lay
        // out captures at their natural-width offsets without going through
        // the `jit_make_closure` FFI. Closure spec §14.6 (H6.5) moves this
        // ABOVE `build_content_addressed_program` so the layouts propagate
        // through the `ContentAddressedProgram` → `LinkedProgram` →
        // `BytecodeProgram` path into the VM's producer.
        //
        // Track A.1C.2: the compiler derives per-capture `CaptureKind`s
        // from the source binding form (see `compile_expr_closure`) and
        // stores them in `closure_capture_kinds`. For each closure literal
        // we rebuild the layout so the `capture_kinds` vector reflects
        // those kinds AND the `owned_mutable_capture_mask` /
        // `shared_capture_mask` bits are flipped for the corresponding
        // capture indices. `op_make_closure` reads those masks to pick
        // the per-capture allocation discipline:
        //   * `CaptureKind::Immutable`   — write the capture bits as-is
        //     at the typed offset.
        //   * `CaptureKind::OwnedMutable` — `Box::into_raw` a fresh
        //     `Box<ValueWord>` around the stack value, write the pointer.
        //   * `CaptureKind::Shared`       — the stack value carries the
        //     raw `*const SharedCell` pointer bits of a previously-
        //     promoted outer slot. `op_make_closure` does
        //     `Arc::increment_strong_count` to give the closure its own
        //     refcount share, then writes the same pointer.
        //
        // This was gated to "masks stay zero" during A.1C partial so the
        // legacy `HeapValue::Closure + SharedCell` fallback could keep
        // running while the compiler migration was incomplete. With
        // A.1C.2 rerouting the outer-scope var lifecycle onto
        // `AllocSharedLocal` / `LoadSharedLocal` / `StoreSharedLocal` /
        // `DropSharedLocal` and the closure-body reads/writes onto
        // `Load/StoreSharedCapture` and `Load/StoreOwnedMutableCapture`,
        // the Raw-path guard can flip bits freely — there is no longer
        // any SharedCell-wrapped ValueWord sitting on the stack at
        // closure-creation time.
        {
            use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
            let total_fns = self.program.functions.len();
            let mut layouts: Vec<Option<std::sync::Arc<ClosureLayout>>> = vec![None; total_fns];
            // Map function index → per-capture CaptureKind vector.
            let kinds_by_fn: std::collections::HashMap<u16, &Vec<CaptureKind>> = self
                .closure_capture_kinds
                .iter()
                .map(|(fid, kinds)| (*fid, kinds))
                .collect();
            for (fn_idx, type_id) in self.closure_type_ids.iter().copied() {
                if let Some(registry_layout) = self.closure_registry.get(type_id) {
                    if (fn_idx as usize) < total_fns {
                        // Track A.1C.3: authoritative per-function kinds.
                        // Both `Shared` AND `OwnedMutable` captures flip
                        // their corresponding mask bits; `op_make_closure`
                        // allocates `Box::into_raw(Box::new(initial))` for
                        // OwnedMutable slots and `Arc::into_raw(Arc::new(
                        // parking_lot::Mutex<ValueWord>))` / `Arc::increment_
                        // strong_count` for Shared slots. Module-binding
                        // `var` captures (migrated in A.1C.3) are also
                        // Shared and follow the same closure-side
                        // allocation discipline; the outer-scope promotion
                        // emits `AllocSharedModuleBinding` (vs.
                        // `AllocSharedLocal` for locals).
                        let per_fn_kinds = kinds_by_fn.get(&fn_idx);
                        let layout_arc = if let Some(kinds) = per_fn_kinds
                            && kinds.len() == registry_layout.capture_types.len()
                        {
                            let rebuilt = ClosureLayout::from_capture_types(
                                &registry_layout.capture_types,
                                kinds,
                            );
                            // Preserve the authoritative per-capture
                            // `capture_kinds` for diagnostics and
                            // A.1D/E JIT lowering.
                            let mut rebuilt = rebuilt;
                            rebuilt.capture_kinds = (*kinds).clone();
                            std::sync::Arc::new(rebuilt)
                        } else {
                            std::sync::Arc::new(registry_layout.clone())
                        };
                        layouts[fn_idx as usize] = Some(layout_arc);
                    }
                }
            }
            self.program.closure_function_layouts = layouts;
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
        // NOTE: root's imports are registered INSIDE `compile()` after the
        // `__main__` blob builder starts, so any emitted Load/Store for
        // namespace-alias bindings (e.g. `use std::core::set` creates a
        // runtime copy from canonical binding `std::core::set` to alias
        // binding `set`) lands inside `__main__`. Registering them here —
        // before `compile()` opens the `__main__` blob — would leave those
        // instructions in an unreachable gap between module bodies and
        // `__main__`'s entry point.

        // Strip import items from root program (imports already resolved via graph)
        let mut stripped_program = root_program.clone();
        stripped_program
            .items
            .retain(|item| !matches!(item, shape_ast::ast::Item::Import(..)));

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
