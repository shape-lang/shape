//! Closure (function expression) compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use crate::compiler::monomorphization::type_resolution::concrete_type_for_expr;
use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
use shape_ast::ast::type_path::TypePath;
use shape_ast::ast::{
    DestructurePattern, Expr, FunctionDef, FunctionParameter, Span, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_runtime::type_system::{BindingFact, InferenceFacts, Type};
use shape_value::v2::concrete_type::{ClosureTypeId, ConcreteType};
use std::collections::{BTreeSet, HashMap, HashSet};

use super::super::BytecodeCompiler;
use crate::compiler::ClosureCallsiteHint;

fn container_kind_from_concrete_type(
    ct: &ConcreteType,
) -> Option<crate::compiler::mutation_writeback::ContainerKind> {
    use crate::compiler::mutation_writeback::ContainerKind;
    match ct {
        ConcreteType::HashMap(_, _) => Some(ContainerKind::HashMap),
        ConcreteType::HashSet(_) => Some(ContainerKind::HashSet),
        ConcreteType::Deque(_) => Some(ContainerKind::Deque),
        ConcreteType::PriorityQueue => Some(ContainerKind::PriorityQueue),
        ConcreteType::Array(_) => Some(ContainerKind::Array),
        _ => None,
    }
}

fn collect_static_mut_self_container_captures(
    compiler: &BytecodeCompiler,
    body: &[shape_ast::ast::Statement],
    captured_vars: &BTreeSet<String>,
) -> BTreeSet<String> {
    use shape_ast::ast::{BlockItem, Expr, Statement};

    fn note_if_static_mut_self_capture(
        compiler: &BytecodeCompiler,
        captured_vars: &BTreeSet<String>,
        receiver: &Expr,
        method: &str,
        out: &mut BTreeSet<String>,
    ) {
        let Expr::Identifier(name, _) = receiver else {
            return;
        };
        if !captured_vars.contains(name) {
            return;
        }
        let ident = Expr::Identifier(name.clone(), Span::DUMMY);
        let Some(container_kind) = concrete_type_for_expr(compiler, &ident)
            .as_ref()
            .and_then(container_kind_from_concrete_type)
        else {
            return;
        };
        if container_kind.is_mut_self_method(method) {
            out.insert(name.clone());
        }
    }

    fn scan_expr(
        compiler: &BytecodeCompiler,
        captured_vars: &BTreeSet<String>,
        expr: &Expr,
        out: &mut BTreeSet<String>,
    ) {
        match expr {
            Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                note_if_static_mut_self_capture(compiler, captured_vars, receiver, method, out);
                scan_expr(compiler, captured_vars, receiver, out);
                for arg in args {
                    scan_expr(compiler, captured_vars, arg, out);
                }
            }
            Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => {
                for arg in args {
                    scan_expr(compiler, captured_vars, arg, out);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                scan_expr(compiler, captured_vars, left, out);
                scan_expr(compiler, captured_vars, right, out);
            }
            Expr::UnaryOp { operand, .. } | Expr::Reference { expr: operand, .. } => {
                scan_expr(compiler, captured_vars, operand, out);
            }
            Expr::Array(elements, _) => {
                for element in elements {
                    scan_expr(compiler, captured_vars, element, out);
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                scan_expr(compiler, captured_vars, object, out);
                scan_expr(compiler, captured_vars, index, out);
            }
            Expr::PropertyAccess { object, .. } => {
                scan_expr(compiler, captured_vars, object, out);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                scan_expr(compiler, captured_vars, condition, out);
                scan_expr(compiler, captured_vars, then_expr, out);
                if let Some(else_expr) = else_expr {
                    scan_expr(compiler, captured_vars, else_expr, out);
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    scan_stmt(compiler, captured_vars, stmt, out);
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    scan_block_item(compiler, captured_vars, item, out);
                }
            }
            Expr::If(if_expr, _) => {
                scan_expr(compiler, captured_vars, &if_expr.condition, out);
                scan_expr(compiler, captured_vars, &if_expr.then_branch, out);
                if let Some(else_branch) = &if_expr.else_branch {
                    scan_expr(compiler, captured_vars, else_branch, out);
                }
            }
            Expr::While(while_expr, _) => {
                scan_expr(compiler, captured_vars, &while_expr.condition, out);
                scan_expr(compiler, captured_vars, &while_expr.body, out);
            }
            Expr::For(for_expr, _) => {
                scan_expr(compiler, captured_vars, &for_expr.iterable, out);
                scan_expr(compiler, captured_vars, &for_expr.body, out);
            }
            Expr::Loop(loop_expr, _) => scan_expr(compiler, captured_vars, &loop_expr.body, out),
            Expr::Match(match_expr, _) => {
                scan_expr(compiler, captured_vars, &match_expr.scrutinee, out);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        scan_expr(compiler, captured_vars, guard, out);
                    }
                    scan_expr(compiler, captured_vars, &arm.body, out);
                }
            }
            Expr::Return(Some(expr), _) | Expr::Await(expr, _) => {
                scan_expr(compiler, captured_vars, expr, out);
            }
            _ => {}
        }
    }

    fn scan_block_item(
        compiler: &BytecodeCompiler,
        captured_vars: &BTreeSet<String>,
        item: &BlockItem,
        out: &mut BTreeSet<String>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let Some(value) = &decl.value {
                    scan_expr(compiler, captured_vars, value, out);
                }
            }
            BlockItem::Assignment(asgn) => scan_expr(compiler, captured_vars, &asgn.value, out),
            BlockItem::Statement(stmt) => scan_stmt(compiler, captured_vars, stmt, out),
            BlockItem::Expression(expr) => scan_expr(compiler, captured_vars, expr, out),
        }
    }

    fn scan_stmt(
        compiler: &BytecodeCompiler,
        captured_vars: &BTreeSet<String>,
        stmt: &Statement,
        out: &mut BTreeSet<String>,
    ) {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                scan_expr(compiler, captured_vars, expr, out);
            }
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    scan_expr(compiler, captured_vars, value, out);
                }
            }
            Statement::Assignment(asgn, _) => scan_expr(compiler, captured_vars, &asgn.value, out),
            Statement::For(for_stmt, _) => {
                for stmt in &for_stmt.body {
                    scan_stmt(compiler, captured_vars, stmt, out);
                }
            }
            Statement::While(while_stmt, _) => {
                scan_expr(compiler, captured_vars, &while_stmt.condition, out);
                for stmt in &while_stmt.body {
                    scan_stmt(compiler, captured_vars, stmt, out);
                }
            }
            Statement::If(if_stmt, _) => {
                scan_expr(compiler, captured_vars, &if_stmt.condition, out);
                for stmt in &if_stmt.then_body {
                    scan_stmt(compiler, captured_vars, stmt, out);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        scan_stmt(compiler, captured_vars, stmt, out);
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = BTreeSet::new();
    for stmt in body {
        scan_stmt(compiler, captured_vars, stmt, &mut out);
    }
    out
}

/// Wave 1a PART A: infer a `TypeAnnotation` for a single call-site argument
/// expression, conservatively. Returns `None` when the argument's type cannot
/// be determined structurally (the corresponding param slot then stays
/// unannotated — no fabrication, no `any`).
///
/// Only literal / structurally-obvious forms are inferred. This is the
/// CALL-SITE side of the bidirectional flow `let f = |a, b| a + b; f(2, 3)`:
/// `f(2, 3)` yields `[Some(int), Some(int)]`, which seeds the closure params.
///
/// `int` and `number` are kept distinct (`2` → int, `2.0` → number) so a
/// later conflicting site (`f(2.0)`) is detected as a conflict rather than
/// silently unified.
pub(crate) fn infer_callsite_arg_type(arg: &Expr) -> Option<TypeAnnotation> {
    use shape_ast::ast::{Literal, UnaryOp};
    match arg {
        Expr::Literal(lit, _) => match lit {
            Literal::Int(_) => Some(TypeAnnotation::Basic("int".to_string())),
            Literal::Number(_) => Some(TypeAnnotation::Basic("number".to_string())),
            Literal::Bool(_) => Some(TypeAnnotation::Basic("bool".to_string())),
            Literal::String(_) => Some(TypeAnnotation::Basic("string".to_string())),
            _ => None,
        },
        // `-2` / `-2.0`: the unary-minus preserves the operand's numeric kind.
        Expr::UnaryOp {
            op: UnaryOp::Neg,
            operand,
            ..
        } => match operand.as_ref() {
            Expr::Literal(Literal::Int(_), _) => Some(TypeAnnotation::Basic("int".to_string())),
            Expr::Literal(Literal::Number(_), _) => {
                Some(TypeAnnotation::Basic("number".to_string()))
            }
            _ => None,
        },
        // `!flag`: a boolean. `not` on a non-bool is itself a strict error
        // raised elsewhere; here we only claim a type when it is structurally
        // a bool-producing op.
        Expr::UnaryOp {
            op: UnaryOp::Not, ..
        } => Some(TypeAnnotation::Basic("bool".to_string())),
        Expr::Array(elements, _) => {
            let mut iter = elements.iter();
            let first = iter.next()?;
            let element_type = infer_callsite_arg_type(first)?;
            for element in iter {
                match infer_callsite_arg_type(element) {
                    Some(next_type) if next_type == element_type => {}
                    _ => return None,
                }
            }
            Some(TypeAnnotation::Array(Box::new(element_type)))
        }
        _ => None,
    }
}

fn array_element_annotation(ann: &TypeAnnotation) -> Option<&TypeAnnotation> {
    match ann {
        TypeAnnotation::Array(inner) => Some(inner),
        TypeAnnotation::Generic { name, args }
            if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 =>
        {
            args.first()
        }
        _ => None,
    }
}

fn insert_closure_array_binding_fact(
    facts: &mut HashMap<Span, BindingFact>,
    name: &str,
    binder_span: Span,
    ann: &TypeAnnotation,
) {
    if binder_span.is_dummy() {
        return;
    }
    facts.insert(
        binder_span,
        BindingFact {
            name: name.to_string(),
            binder_span,
            initializer_span: None,
            ty: Type::Concrete(ann.clone()),
        },
    );
}

fn collect_closure_array_binding_facts(
    pattern: &DestructurePattern,
    ann: &TypeAnnotation,
    facts: &mut HashMap<Span, BindingFact>,
) {
    let Some(element_ann) = array_element_annotation(ann) else {
        return;
    };
    let DestructurePattern::Array(items) = pattern else {
        return;
    };
    for item in items {
        match item {
            DestructurePattern::Identifier(name, span) => {
                insert_closure_array_binding_fact(facts, name, *span, element_ann);
            }
            DestructurePattern::Array(_) => {
                collect_closure_array_binding_facts(item, element_ann, facts);
            }
            _ => {}
        }
    }
}

fn closure_array_binding_facts(params: &[FunctionParameter]) -> HashMap<Span, BindingFact> {
    let mut facts = HashMap::new();
    for param in params {
        if let Some(annotation) = param.type_annotation.as_ref() {
            collect_closure_array_binding_facts(&param.pattern, annotation, &mut facts);
        }
    }
    facts
}

fn inference_facts_with_closure_binding_facts(
    base: &InferenceFacts,
    extra: HashMap<Span, BindingFact>,
) -> Option<InferenceFacts> {
    if extra.is_empty() {
        return None;
    }
    let mut binding_facts = base.binding_facts().clone();
    let mut changed = false;
    for (span, fact) in extra {
        match binding_facts.get(&span) {
            Some(existing) if existing.ty == fact.ty => {}
            _ => {
                binding_facts.insert(span, fact);
                changed = true;
            }
        }
    }
    changed.then(|| {
        InferenceFacts::with_binding_facts(
            base.top_level_types().clone(),
            base.expression_types().clone(),
            binding_facts,
        )
    })
}

fn closure_binding_fact_types_by_name(facts: &HashMap<Span, BindingFact>) -> HashMap<String, Type> {
    let mut out = HashMap::new();
    let mut ambiguous = HashSet::new();
    for fact in facts.values() {
        if out.contains_key(&fact.name) {
            ambiguous.insert(fact.name.clone());
        } else {
            out.insert(fact.name.clone(), fact.ty.clone());
        }
    }
    for name in ambiguous {
        out.remove(&name);
    }
    out
}

fn collect_closure_expr_type_overrides(
    body: &[shape_ast::ast::Statement],
    binding_types: &HashMap<String, Type>,
) -> HashMap<Span, Type> {
    use shape_runtime::visitor::{Visitor, walk_expr, walk_stmt};

    fn remove_destructure_bindings(
        active: &mut HashMap<String, Type>,
        pattern: &DestructurePattern,
    ) {
        for (name, _) in pattern.get_bindings() {
            active.remove(&name);
        }
    }

    fn remove_value_bindings(
        active: &mut HashMap<String, Type>,
        pattern: &shape_ast::ast::Pattern,
    ) {
        for (name, _) in pattern.get_bindings() {
            active.remove(&name);
        }
    }

    fn remove_named_binding(active: &mut HashMap<String, Type>, name: &str) {
        active.remove(name);
    }

    struct Collector {
        active: HashMap<String, Type>,
        out: HashMap<Span, Type>,
    }

    impl Visitor for Collector {
        fn visit_expr_identifier(&mut self, expr: &Expr, span: Span) -> bool {
            if let Expr::Identifier(name, _) = expr {
                if !span.is_dummy() {
                    if let Some(ty) = self.active.get(name) {
                        self.out.insert(span, ty.clone());
                    }
                }
            }
            true
        }

        fn visit_expr_function_expr(&mut self, _expr: &Expr, _span: Span) -> bool {
            false
        }

        fn visit_expr_list_comprehension(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::ListComprehension(comp, _) = expr else {
                return true;
            };
            let saved = self.active.clone();
            for clause in &comp.clauses {
                walk_expr(self, &clause.iterable);
                remove_destructure_bindings(&mut self.active, &clause.pattern);
                if let Some(filter) = &clause.filter {
                    walk_expr(self, filter);
                }
            }
            walk_expr(self, &comp.element);
            self.active = saved;
            false
        }

        fn visit_expr_block(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::Block(block, _) = expr else {
                return true;
            };
            let saved = self.active.clone();
            for item in &block.items {
                match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &decl.value {
                            walk_expr(self, value);
                        }
                        remove_destructure_bindings(&mut self.active, &decl.pattern);
                    }
                    shape_ast::ast::BlockItem::Assignment(assign) => walk_expr(self, &assign.value),
                    shape_ast::ast::BlockItem::Statement(stmt) => walk_stmt(self, stmt),
                    shape_ast::ast::BlockItem::Expression(expr) => walk_expr(self, expr),
                }
            }
            self.active = saved;
            false
        }

        fn visit_expr_for(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::For(for_expr, _) = expr else {
                return true;
            };
            walk_expr(self, &for_expr.iterable);
            let saved = self.active.clone();
            remove_value_bindings(&mut self.active, &for_expr.pattern);
            walk_expr(self, &for_expr.body);
            self.active = saved;
            false
        }

        fn visit_expr_let(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::Let(let_expr, _) = expr else {
                return true;
            };
            if let Some(value) = &let_expr.value {
                walk_expr(self, value);
            }
            let saved = self.active.clone();
            remove_value_bindings(&mut self.active, &let_expr.pattern);
            walk_expr(self, &let_expr.body);
            self.active = saved;
            false
        }

        fn visit_expr_match(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::Match(match_expr, _) = expr else {
                return true;
            };
            walk_expr(self, &match_expr.scrutinee);
            for arm in &match_expr.arms {
                let saved = self.active.clone();
                remove_value_bindings(&mut self.active, &arm.pattern);
                if let Some(guard) = &arm.guard {
                    walk_expr(self, guard);
                }
                walk_expr(self, &arm.body);
                self.active = saved;
            }
            false
        }

        fn visit_expr_from_query(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::FromQuery(from_query, _) = expr else {
                return true;
            };
            walk_expr(self, &from_query.source);
            let saved = self.active.clone();
            remove_named_binding(&mut self.active, &from_query.variable);
            for clause in &from_query.clauses {
                match clause {
                    shape_ast::ast::QueryClause::Where(pred) => walk_expr(self, pred),
                    shape_ast::ast::QueryClause::OrderBy(specs) => {
                        for spec in specs {
                            walk_expr(self, &spec.key);
                        }
                    }
                    shape_ast::ast::QueryClause::GroupBy {
                        element,
                        key,
                        into_var,
                    } => {
                        walk_expr(self, element);
                        walk_expr(self, key);
                        if let Some(name) = into_var {
                            remove_named_binding(&mut self.active, name);
                        }
                    }
                    shape_ast::ast::QueryClause::Join {
                        variable,
                        source,
                        left_key,
                        right_key,
                        into_var,
                    } => {
                        walk_expr(self, source);
                        walk_expr(self, left_key);
                        remove_named_binding(&mut self.active, variable);
                        walk_expr(self, right_key);
                        if let Some(name) = into_var {
                            remove_named_binding(&mut self.active, name);
                        }
                    }
                    shape_ast::ast::QueryClause::Let { variable, value } => {
                        walk_expr(self, value);
                        remove_named_binding(&mut self.active, variable);
                    }
                }
            }
            walk_expr(self, &from_query.select);
            self.active = saved;
            false
        }

        fn visit_expr_async_let(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::AsyncLet(async_let, _) = expr else {
                return true;
            };
            walk_expr(self, &async_let.expr);
            remove_named_binding(&mut self.active, &async_let.name);
            false
        }

        fn visit_expr_comptime_for(&mut self, expr: &Expr, _span: Span) -> bool {
            let Expr::ComptimeFor(comptime_for, _) = expr else {
                return true;
            };
            walk_expr(self, &comptime_for.iterable);
            let saved = self.active.clone();
            remove_named_binding(&mut self.active, &comptime_for.variable);
            for stmt in &comptime_for.body {
                walk_stmt(self, stmt);
            }
            self.active = saved;
            false
        }

        fn visit_stmt(&mut self, stmt: &shape_ast::ast::Statement) -> bool {
            use shape_ast::ast::{ForInit, Statement};
            match stmt {
                Statement::VariableDecl(decl, _) => {
                    if let Some(value) = &decl.value {
                        walk_expr(self, value);
                    }
                    remove_destructure_bindings(&mut self.active, &decl.pattern);
                    false
                }
                Statement::For(for_stmt, _) => {
                    let saved = self.active.clone();
                    match &for_stmt.init {
                        ForInit::ForIn { pattern, iter } => {
                            walk_expr(self, iter);
                            remove_destructure_bindings(&mut self.active, pattern);
                        }
                        ForInit::ForC {
                            init,
                            condition,
                            update,
                        } => {
                            walk_stmt(self, init);
                            walk_expr(self, condition);
                            walk_expr(self, update);
                        }
                    }
                    for stmt in &for_stmt.body {
                        walk_stmt(self, stmt);
                    }
                    self.active = saved;
                    false
                }
                Statement::If(if_stmt, _) => {
                    walk_expr(self, &if_stmt.condition);
                    let saved = self.active.clone();
                    for stmt in &if_stmt.then_body {
                        walk_stmt(self, stmt);
                    }
                    self.active = saved.clone();
                    if let Some(else_body) = &if_stmt.else_body {
                        for stmt in else_body {
                            walk_stmt(self, stmt);
                        }
                    }
                    self.active = saved;
                    false
                }
                Statement::While(while_stmt, _) => {
                    walk_expr(self, &while_stmt.condition);
                    let saved = self.active.clone();
                    for stmt in &while_stmt.body {
                        walk_stmt(self, stmt);
                    }
                    self.active = saved;
                    false
                }
                _ => true,
            }
        }
    }

    let mut collector = Collector {
        active: binding_types.clone(),
        out: HashMap::new(),
    };
    for stmt in body {
        walk_stmt(&mut collector, stmt);
    }
    collector.out
}

fn annotation_contains_unknown(ann: &TypeAnnotation) -> bool {
    match ann {
        TypeAnnotation::Basic(name) => name == "unknown",
        TypeAnnotation::Array(inner) => annotation_contains_unknown(inner),
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => items.iter().any(annotation_contains_unknown),
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|field| annotation_contains_unknown(&field.type_annotation)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|param| annotation_contains_unknown(&param.type_annotation))
                || annotation_contains_unknown(returns)
        }
        TypeAnnotation::Generic { args, .. } => args.iter().any(annotation_contains_unknown),
        TypeAnnotation::Borrow { inner, .. } => annotation_contains_unknown(inner),
        TypeAnnotation::Reference(_)
        | TypeAnnotation::Dyn(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => false,
    }
}

fn type_to_concrete_annotation(ty: &Type) -> Option<TypeAnnotation> {
    let ann = match ty {
        Type::Variable(_) | Type::Constrained { .. } => return None,
        Type::Concrete(ann) => ann.clone(),
        Type::Generic { .. } | Type::Function { .. } => ty.to_annotation()?,
    };
    if annotation_contains_unknown(&ann) {
        None
    } else {
        Some(ann)
    }
}

fn function_type_param_hints(
    ty: &Type,
    expected_arity: usize,
) -> Option<Vec<Option<TypeAnnotation>>> {
    match ty.canonicalize() {
        Type::Function { params, .. } if params.len() == expected_arity => {
            let hints: Vec<Option<TypeAnnotation>> =
                params.iter().map(type_to_concrete_annotation).collect();
            if hints.iter().any(Option::is_some) {
                Some(hints)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Wave 1a PART A: merge a freshly-observed call site's per-arg inferred types
/// into the accumulated hint for a binding name.
///
/// Soundness rules (strict-typing core):
/// * Differing concrete annotations at the same slot ⇒ `Conflict` (never pick
///   one). `int` vs `number` differ, so they conflict.
/// * `None` at a slot is "no info from this site"; it neither confirms nor
///   conflicts — a later/earlier `Some` at that slot is kept.
/// * Differing arities across call sites ⇒ `Conflict` (the binding is being
///   used inconsistently; surface it rather than guess).
fn merge_callsite_hint(
    existing: Option<ClosureCallsiteHint>,
    observed: Vec<Option<TypeAnnotation>>,
) -> ClosureCallsiteHint {
    match existing {
        None => ClosureCallsiteHint::Types(observed),
        Some(ClosureCallsiteHint::Conflict) => ClosureCallsiteHint::Conflict,
        Some(ClosureCallsiteHint::Types(prev)) => {
            if prev.len() != observed.len() {
                return ClosureCallsiteHint::Conflict;
            }
            let mut merged = Vec::with_capacity(prev.len());
            for (a, b) in prev.into_iter().zip(observed.into_iter()) {
                match (a, b) {
                    (Some(ta), Some(tb)) => {
                        if ta == tb {
                            merged.push(Some(ta));
                        } else {
                            // int != number, and any two distinct annotations
                            // at the same slot are a genuine conflict.
                            return ClosureCallsiteHint::Conflict;
                        }
                    }
                    (Some(t), None) | (None, Some(t)) => merged.push(Some(t)),
                    (None, None) => merged.push(None),
                }
            }
            ClosureCallsiteHint::Types(merged)
        }
    }
}

fn merge_callable_arities(arities: impl IntoIterator<Item = Option<usize>>) -> Option<usize> {
    let mut merged = None;
    for arity in arities {
        let arity = arity?;
        match merged {
            None => merged = Some(arity),
            Some(prev) if prev == arity => {}
            Some(_) => return None,
        }
    }
    merged
}

/// Returns the callable arity when an expression statically selects only
/// closure literals of one arity. Signature compatibility is still enforced by
/// the runtime type checker; this is only a producer-side hint classifier for
/// closure parameter facts.
pub(crate) fn callable_selection_arity(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::FunctionExpr { params, .. } => Some(params.len()),
        Expr::If(if_expr, _) => {
            let else_branch = if_expr.else_branch.as_ref()?;
            merge_callable_arities([
                callable_selection_arity(&if_expr.then_branch),
                callable_selection_arity(else_branch),
            ])
        }
        Expr::Conditional {
            then_expr,
            else_expr,
            ..
        } => {
            let else_expr = else_expr.as_ref()?;
            merge_callable_arities([
                callable_selection_arity(then_expr),
                callable_selection_arity(else_expr),
            ])
        }
        Expr::Match(match_expr, _) => {
            if match_expr.arms.is_empty() {
                return None;
            }
            merge_callable_arities(
                match_expr
                    .arms
                    .iter()
                    .map(|arm| callable_selection_arity(&arm.body)),
            )
        }
        Expr::Block(block, _) => block.items.last().and_then(|item| match item {
            shape_ast::ast::BlockItem::Expression(expr) => callable_selection_arity(expr),
            shape_ast::ast::BlockItem::Statement(shape_ast::ast::Statement::Expression(
                expr,
                _,
            )) => callable_selection_arity(expr),
            _ => None,
        }),
        Expr::Return(Some(expr), _) => callable_selection_arity(expr),
        _ => None,
    }
}

fn collect_return_callable_arities_from_stmt(
    stmt: &shape_ast::ast::Statement,
    arities: &mut Vec<Option<usize>>,
) {
    use shape_ast::ast::Statement;
    match stmt {
        Statement::Return(Some(expr), _) => arities.push(callable_selection_arity(expr)),
        Statement::If(if_stmt, _) => {
            for stmt in &if_stmt.then_body {
                collect_return_callable_arities_from_stmt(stmt, arities);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for stmt in else_body {
                    collect_return_callable_arities_from_stmt(stmt, arities);
                }
            }
        }
        Statement::For(for_stmt, _) => {
            for stmt in &for_stmt.body {
                collect_return_callable_arities_from_stmt(stmt, arities);
            }
        }
        Statement::While(while_stmt, _) => {
            for stmt in &while_stmt.body {
                collect_return_callable_arities_from_stmt(stmt, arities);
            }
        }
        Statement::Expression(Expr::Return(Some(expr), _), _) => {
            arities.push(callable_selection_arity(expr));
        }
        _ => {}
    }
}

fn function_return_callable_arity(func: &FunctionDef) -> Option<usize> {
    let mut explicit_return_arities = Vec::new();
    for stmt in &func.body {
        collect_return_callable_arities_from_stmt(stmt, &mut explicit_return_arities);
    }
    if !explicit_return_arities.is_empty() {
        return merge_callable_arities(explicit_return_arities);
    }

    match func.body.last() {
        Some(shape_ast::ast::Statement::Expression(expr, _)) => callable_selection_arity(expr),
        _ => None,
    }
}

/// Wave 1a PART A: whole-program pre-pass that, for every binding whose
/// initializer is a closure literal, scans the program for DIRECT calls
/// `name(args)` and records the per-arg call-site argument types so
/// `compile_expr_closure` can seed the closure's unannotated params.
///
/// This is the producer side of the bidirectional flow; the consumer is
/// `compile_expr_closure` (keyed on `pending_variable_name`).
///
/// Soundness: a name bound to a closure literal in MORE THAN ONE place
/// (shadowing across scopes) is marked `Conflict` up front — a single
/// name-keyed hint cannot soundly serve two distinct closures. Likewise a
/// name called with conflicting arg types is `Conflict`. In both cases the
/// hint is not applied and the closure keeps its existing behavior.
pub(crate) fn collect_closure_callsite_param_hints(
    program: &shape_ast::ast::Program,
) -> std::collections::HashMap<String, ClosureCallsiteHint> {
    use shape_ast::ast::{BlockItem, Item, Statement};
    use std::collections::{HashMap, HashSet};

    // ----- Pass 1: collect closure-bound names (and shadowing) -----
    //
    // A single recursive expr/stmt walker traverses BOTH statement-level and
    // expression-level control flow (`Expr::For`/`While`/`If`/`Loop`/`Block`/
    // `Match` and `BlockItem`), because a top-level `for`/`if`/block is parsed
    // as an EXPRESSION at the item level, and bindings/calls can live anywhere
    // inside.

    let mut closure_binding_names: HashSet<String> = HashSet::new();
    let mut shadowed_names: HashSet<String> = HashSet::new();
    let callable_return_functions: HashSet<String> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(func, _) if function_return_callable_arity(func).is_some() => {
                Some(func.name.clone())
            }
            _ => None,
        })
        .collect();
    let mut callable_return_bindings: HashMap<String, String> = HashMap::new();

    fn note_binding(
        name: &str,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        if !closure_names.insert(name.to_string()) {
            shadowed.insert(name.to_string());
        }
    }

    fn note_decl_value(
        name: &str,
        value: &Expr,
        callable_return_functions: &HashSet<String>,
        callable_return_bindings: &mut HashMap<String, String>,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        if callable_selection_arity(value).is_some() {
            note_binding(name, closure_names, shadowed);
            return;
        }
        if let Expr::FunctionCall {
            name: callee_name, ..
        } = value
        {
            if callable_return_functions.contains(callee_name) {
                note_binding(name, closure_names, shadowed);
                callable_return_bindings.insert(name.to_string(), callee_name.clone());
            }
        }
    }

    fn bind_stmt(
        stmt: &Statement,
        callable_return_functions: &HashSet<String>,
        callable_return_bindings: &mut HashMap<String, String>,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match stmt {
            Statement::VariableDecl(decl, _) => {
                if let (Some(name), Some(value)) =
                    (decl.pattern.as_identifier(), decl.value.as_ref())
                {
                    note_decl_value(
                        name,
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                    bind_expr(
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Statement::Assignment(asgn, _) => bind_expr(
                &asgn.value,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Statement::Expression(e, _) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Statement::Return(Some(e), _) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Statement::For(f, _) => {
                for s in &f.body {
                    bind_stmt(
                        s,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Statement::While(w, _) => {
                for s in &w.body {
                    bind_stmt(
                        s,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Statement::If(i, _) => {
                for s in &i.then_body {
                    bind_stmt(
                        s,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
                if let Some(else_body) = &i.else_body {
                    for s in else_body {
                        bind_stmt(
                            s,
                            callable_return_functions,
                            callable_return_bindings,
                            closure_names,
                            shadowed,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn bind_block_item(
        item: &BlockItem,
        callable_return_functions: &HashSet<String>,
        callable_return_bindings: &mut HashMap<String, String>,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let (Some(name), Some(value)) =
                    (decl.pattern.as_identifier(), decl.value.as_ref())
                {
                    note_decl_value(
                        name,
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                    bind_expr(
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            BlockItem::Assignment(asgn) => bind_expr(
                &asgn.value,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            BlockItem::Statement(stmt) => bind_stmt(
                stmt,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            BlockItem::Expression(e) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
        }
    }

    fn bind_expr(
        expr: &Expr,
        callable_return_functions: &HashSet<String>,
        callable_return_bindings: &mut HashMap<String, String>,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match expr {
            Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => {
                for a in args {
                    bind_expr(
                        a,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                bind_expr(
                    receiver,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                for a in args {
                    bind_expr(
                        a,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                bind_expr(
                    left,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    right,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
            }
            Expr::UnaryOp { operand, .. } => bind_expr(
                operand,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Expr::Reference { expr, .. } => bind_expr(
                expr,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Expr::Array(elems, _) => {
                for e in elems {
                    bind_expr(
                        e,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                bind_expr(
                    object,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    index,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
            }
            Expr::PropertyAccess { object, .. } => bind_expr(
                object,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                bind_expr(
                    condition,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    then_expr,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                if let Some(e) = else_expr {
                    bind_expr(
                        e,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for s in body {
                    bind_stmt(
                        s,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::Block(block, _) => {
                for it in &block.items {
                    bind_block_item(
                        it,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::If(i, _) => {
                bind_expr(
                    &i.condition,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    &i.then_branch,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                if let Some(e) = &i.else_branch {
                    bind_expr(
                        e,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::While(w, _) => {
                bind_expr(
                    &w.condition,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    &w.body,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
            }
            Expr::For(f, _) => {
                bind_expr(
                    &f.iterable,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                bind_expr(
                    &f.body,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
            }
            Expr::Loop(l, _) => bind_expr(
                &l.body,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Expr::Match(m, _) => {
                bind_expr(
                    &m.scrutinee,
                    callable_return_functions,
                    callable_return_bindings,
                    closure_names,
                    shadowed,
                );
                for arm in &m.arms {
                    bind_expr(
                        &arm.body,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Expr::Return(Some(e), _) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Expr::Await(e, _) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            _ => {}
        }
    }

    fn bind_item(
        item: &Item,
        callable_return_functions: &HashSet<String>,
        callable_return_bindings: &mut HashMap<String, String>,
        closure_names: &mut HashSet<String>,
        shadowed: &mut HashSet<String>,
    ) {
        match item {
            Item::Statement(stmt, _) => bind_stmt(
                stmt,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Item::Expression(e, _) => bind_expr(
                e,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Item::Assignment(asgn, _) => bind_expr(
                &asgn.value,
                callable_return_functions,
                callable_return_bindings,
                closure_names,
                shadowed,
            ),
            Item::VariableDecl(decl, _) => {
                if let (Some(name), Some(value)) =
                    (decl.pattern.as_identifier(), decl.value.as_ref())
                {
                    note_decl_value(
                        name,
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                    bind_expr(
                        value,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            Item::Function(func, _) => {
                for s in &func.body {
                    bind_stmt(
                        s,
                        callable_return_functions,
                        callable_return_bindings,
                        closure_names,
                        shadowed,
                    );
                }
            }
            _ => {}
        }
    }

    for item in &program.items {
        bind_item(
            item,
            &callable_return_functions,
            &mut callable_return_bindings,
            &mut closure_binding_names,
            &mut shadowed_names,
        );
    }

    // ----- Pass 2: collect call-site arg types for eligible names -----

    let mut hints: HashMap<String, ClosureCallsiteHint> = HashMap::new();

    fn handle_call(
        name: &str,
        args: &[Expr],
        eligible: &HashSet<String>,
        callable_return_bindings: &HashMap<String, String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        if !eligible.contains(name) {
            return;
        }
        let observed: Vec<Option<TypeAnnotation>> =
            args.iter().map(infer_callsite_arg_type).collect();
        let merged = merge_callsite_hint(hints.remove(name), observed.clone());
        hints.insert(name.to_string(), merged);
        if let Some(producer_name) = callable_return_bindings.get(name) {
            let merged = merge_callsite_hint(hints.remove(producer_name), observed);
            hints.insert(producer_name.clone(), merged);
        }
    }

    fn walk_expr(
        expr: &Expr,
        eligible: &HashSet<String>,
        callable_return_bindings: &HashMap<String, String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match expr {
            Expr::FunctionCall { name, args, .. } => {
                handle_call(name, args, eligible, callable_return_bindings, hints);
                for a in args {
                    walk_expr(a, eligible, callable_return_bindings, hints);
                }
            }
            Expr::QualifiedFunctionCall { args, .. } => {
                for a in args {
                    walk_expr(a, eligible, callable_return_bindings, hints);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                walk_expr(receiver, eligible, callable_return_bindings, hints);
                for a in args {
                    walk_expr(a, eligible, callable_return_bindings, hints);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                walk_expr(left, eligible, callable_return_bindings, hints);
                walk_expr(right, eligible, callable_return_bindings, hints);
            }
            Expr::UnaryOp { operand, .. } => {
                walk_expr(operand, eligible, callable_return_bindings, hints)
            }
            Expr::Reference { expr, .. } => {
                walk_expr(expr, eligible, callable_return_bindings, hints)
            }
            Expr::Array(elems, _) => {
                for e in elems {
                    walk_expr(e, eligible, callable_return_bindings, hints);
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                walk_expr(object, eligible, callable_return_bindings, hints);
                walk_expr(index, eligible, callable_return_bindings, hints);
            }
            Expr::PropertyAccess { object, .. } => {
                walk_expr(object, eligible, callable_return_bindings, hints)
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                walk_expr(condition, eligible, callable_return_bindings, hints);
                walk_expr(then_expr, eligible, callable_return_bindings, hints);
                if let Some(else_e) = else_expr {
                    walk_expr(else_e, eligible, callable_return_bindings, hints);
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for s in body {
                    walk_stmt(s, eligible, callable_return_bindings, hints);
                }
            }
            Expr::Block(block, _) => {
                for it in &block.items {
                    walk_block_item(it, eligible, callable_return_bindings, hints);
                }
            }
            Expr::If(i, _) => {
                walk_expr(&i.condition, eligible, callable_return_bindings, hints);
                walk_expr(&i.then_branch, eligible, callable_return_bindings, hints);
                if let Some(e) = &i.else_branch {
                    walk_expr(e, eligible, callable_return_bindings, hints);
                }
            }
            Expr::While(w, _) => {
                walk_expr(&w.condition, eligible, callable_return_bindings, hints);
                walk_expr(&w.body, eligible, callable_return_bindings, hints);
            }
            Expr::For(f, _) => {
                walk_expr(&f.iterable, eligible, callable_return_bindings, hints);
                walk_expr(&f.body, eligible, callable_return_bindings, hints);
            }
            Expr::Loop(l, _) => walk_expr(&l.body, eligible, callable_return_bindings, hints),
            Expr::Match(m, _) => {
                walk_expr(&m.scrutinee, eligible, callable_return_bindings, hints);
                for arm in &m.arms {
                    walk_expr(&arm.body, eligible, callable_return_bindings, hints);
                }
            }
            Expr::Return(Some(e), _) => walk_expr(e, eligible, callable_return_bindings, hints),
            Expr::Await(e, _) => walk_expr(e, eligible, callable_return_bindings, hints),
            _ => {}
        }
    }

    fn walk_block_item(
        item: &BlockItem,
        eligible: &HashSet<String>,
        callable_return_bindings: &HashMap<String, String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let Some(v) = decl.value.as_ref() {
                    walk_expr(v, eligible, callable_return_bindings, hints);
                }
            }
            BlockItem::Assignment(asgn) => {
                walk_expr(&asgn.value, eligible, callable_return_bindings, hints)
            }
            BlockItem::Statement(stmt) => {
                walk_stmt(stmt, eligible, callable_return_bindings, hints)
            }
            BlockItem::Expression(e) => walk_expr(e, eligible, callable_return_bindings, hints),
        }
    }

    fn walk_stmt(
        stmt: &Statement,
        eligible: &HashSet<String>,
        callable_return_bindings: &HashMap<String, String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match stmt {
            Statement::Expression(e, _) => walk_expr(e, eligible, callable_return_bindings, hints),
            Statement::Return(Some(e), _) => {
                walk_expr(e, eligible, callable_return_bindings, hints)
            }
            Statement::VariableDecl(decl, _) => {
                if let Some(v) = decl.value.as_ref() {
                    walk_expr(v, eligible, callable_return_bindings, hints);
                }
            }
            Statement::Assignment(asgn, _) => {
                walk_expr(&asgn.value, eligible, callable_return_bindings, hints)
            }
            Statement::For(f, _) => {
                for s in &f.body {
                    walk_stmt(s, eligible, callable_return_bindings, hints);
                }
            }
            Statement::While(w, _) => {
                walk_expr(&w.condition, eligible, callable_return_bindings, hints);
                for s in &w.body {
                    walk_stmt(s, eligible, callable_return_bindings, hints);
                }
            }
            Statement::If(i, _) => {
                walk_expr(&i.condition, eligible, callable_return_bindings, hints);
                for s in &i.then_body {
                    walk_stmt(s, eligible, callable_return_bindings, hints);
                }
                if let Some(else_body) = &i.else_body {
                    for s in else_body {
                        walk_stmt(s, eligible, callable_return_bindings, hints);
                    }
                }
            }
            _ => {}
        }
    }

    for item in &program.items {
        match item {
            Item::Statement(stmt, _) => walk_stmt(
                stmt,
                &closure_binding_names,
                &callable_return_bindings,
                &mut hints,
            ),
            Item::Expression(e, _) => walk_expr(
                e,
                &closure_binding_names,
                &callable_return_bindings,
                &mut hints,
            ),
            Item::Assignment(asgn, _) => walk_expr(
                &asgn.value,
                &closure_binding_names,
                &callable_return_bindings,
                &mut hints,
            ),
            Item::VariableDecl(decl, _) => {
                if let Some(v) = decl.value.as_ref() {
                    walk_expr(
                        v,
                        &closure_binding_names,
                        &callable_return_bindings,
                        &mut hints,
                    );
                }
            }
            Item::Function(func, _) => {
                for s in &func.body {
                    walk_stmt(
                        s,
                        &closure_binding_names,
                        &callable_return_bindings,
                        &mut hints,
                    );
                }
            }
            _ => {}
        }
    }

    // Demote shadowed names to Conflict — a single name-keyed hint cannot
    // soundly serve two distinct closure definitions.
    for name in shadowed_names {
        hints.insert(name, ClosureCallsiteHint::Conflict);
    }

    hints
}

/// W21 HOF inference: whole-program pre-pass for factory-returned closures.
///
/// Direct closure bindings are handled by `collect_closure_callsite_param_hints`
/// (`let f = |a| ...; f(1)`). This pass covers the adjacent factory shape:
/// `let f = make_op(...); f(1)`, where `make_op` syntactically returns closure
/// literals. The hint is keyed by `make_op`, because the closure literal is
/// compiled while the producer function body is being compiled.
///
/// Soundness mirrors the direct-binding pass:
/// * only syntactic closure-returning functions are eligible;
/// * only literal / structurally-obvious call arguments produce types;
/// * duplicate result-binding names, arity mismatches, or `int`/`number`
///   disagreements become `Conflict`, so no type is guessed.
pub(crate) fn collect_returned_closure_callsite_param_hints(
    program: &shape_ast::ast::Program,
) -> std::collections::HashMap<String, ClosureCallsiteHint> {
    use shape_ast::ast::{BlockItem, Expr, Item, Statement};
    use std::collections::{HashMap, HashSet};

    fn expr_returns_closure(expr: &Expr) -> bool {
        match expr {
            Expr::FunctionExpr { .. } => true,
            Expr::If(if_expr, _) => {
                expr_returns_closure(&if_expr.then_branch)
                    && if_expr
                        .else_branch
                        .as_ref()
                        .is_some_and(|else_expr| expr_returns_closure(else_expr))
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                expr_returns_closure(then_expr)
                    && else_expr
                        .as_ref()
                        .is_some_and(|else_expr| expr_returns_closure(else_expr))
            }
            Expr::Block(block, _) => block_expr_returns_closure(block),
            Expr::Match(match_expr, _) => {
                !match_expr.arms.is_empty()
                    && match_expr
                        .arms
                        .iter()
                        .all(|arm| expr_returns_closure(&arm.body))
            }
            Expr::Return(Some(expr), _) => expr_returns_closure(expr),
            _ => false,
        }
    }

    fn stmt_tail_returns_closure(stmt: &Statement) -> bool {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                expr_returns_closure(expr)
            }
            Statement::If(if_stmt, _) => {
                block_returns_closure(&if_stmt.then_body)
                    && if_stmt
                        .else_body
                        .as_ref()
                        .is_some_and(|body| block_returns_closure(body))
            }
            _ => false,
        }
    }

    fn block_returns_closure(body: &[Statement]) -> bool {
        body.iter().any(
            |stmt| matches!(stmt, Statement::Return(Some(expr), _) if expr_returns_closure(expr)),
        ) || body
            .last()
            .is_some_and(|stmt| stmt_tail_returns_closure(stmt))
    }

    fn block_expr_returns_closure(block: &shape_ast::ast::BlockExpr) -> bool {
        block.items.iter().any(|item| {
            matches!(
                item,
                BlockItem::Statement(Statement::Return(Some(expr), _))
                    if expr_returns_closure(expr)
            )
        }) || block.items.last().is_some_and(|item| match item {
            BlockItem::Expression(expr) => expr_returns_closure(expr),
            BlockItem::Statement(stmt) => stmt_tail_returns_closure(stmt),
            _ => false,
        })
    }

    fn note_result_binding(
        binding_name: &str,
        function_name: &str,
        eligible_functions: &HashSet<String>,
        result_bindings: &mut HashMap<String, String>,
        conflicted_bindings: &mut HashSet<String>,
    ) {
        if !eligible_functions.contains(function_name) {
            return;
        }
        if result_bindings.contains_key(binding_name) {
            result_bindings.remove(binding_name);
            conflicted_bindings.insert(binding_name.to_string());
            return;
        }
        if !conflicted_bindings.contains(binding_name) {
            result_bindings.insert(binding_name.to_string(), function_name.to_string());
        }
    }

    fn bind_decl(
        decl: &shape_ast::ast::VariableDecl,
        eligible_functions: &HashSet<String>,
        result_bindings: &mut HashMap<String, String>,
        conflicted_bindings: &mut HashSet<String>,
    ) {
        if let (Some(binding_name), Some(value)) =
            (decl.pattern.as_identifier(), decl.value.as_ref())
        {
            if let Expr::FunctionCall { name, .. } = value {
                note_result_binding(
                    binding_name,
                    name,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
            }
            bind_expr(
                value,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            );
        }
    }

    fn bind_stmt(
        stmt: &Statement,
        eligible_functions: &HashSet<String>,
        result_bindings: &mut HashMap<String, String>,
        conflicted_bindings: &mut HashSet<String>,
    ) {
        match stmt {
            Statement::VariableDecl(decl, _) => bind_decl(
                decl,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Statement::Assignment(asgn, _) => bind_expr(
                &asgn.value,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => bind_expr(
                expr,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Statement::For(for_stmt, _) => {
                for stmt in &for_stmt.body {
                    bind_stmt(
                        stmt,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Statement::While(while_stmt, _) => {
                bind_expr(
                    &while_stmt.condition,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                for stmt in &while_stmt.body {
                    bind_stmt(
                        stmt,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Statement::If(if_stmt, _) => {
                bind_expr(
                    &if_stmt.condition,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                for stmt in &if_stmt.then_body {
                    bind_stmt(
                        stmt,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        bind_stmt(
                            stmt,
                            eligible_functions,
                            result_bindings,
                            conflicted_bindings,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn bind_block_item(
        item: &BlockItem,
        eligible_functions: &HashSet<String>,
        result_bindings: &mut HashMap<String, String>,
        conflicted_bindings: &mut HashSet<String>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => bind_decl(
                decl,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            BlockItem::Assignment(asgn) => bind_expr(
                &asgn.value,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            BlockItem::Statement(stmt) => bind_stmt(
                stmt,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            BlockItem::Expression(expr) => bind_expr(
                expr,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
        }
    }

    fn bind_expr(
        expr: &Expr,
        eligible_functions: &HashSet<String>,
        result_bindings: &mut HashMap<String, String>,
        conflicted_bindings: &mut HashSet<String>,
    ) {
        match expr {
            Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => {
                for arg in args {
                    bind_expr(
                        arg,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                bind_expr(
                    receiver,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                for arg in args {
                    bind_expr(
                        arg,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                bind_expr(
                    left,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    right,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
            }
            Expr::UnaryOp { operand, .. } | Expr::Reference { expr: operand, .. } => bind_expr(
                operand,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Expr::Array(elements, _) => {
                for element in elements {
                    bind_expr(
                        element,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                bind_expr(
                    object,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    index,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
            }
            Expr::PropertyAccess { object, .. } => bind_expr(
                object,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                bind_expr(
                    condition,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    then_expr,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                if let Some(else_expr) = else_expr {
                    bind_expr(
                        else_expr,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    bind_stmt(
                        stmt,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    bind_block_item(
                        item,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::If(if_expr, _) => {
                bind_expr(
                    &if_expr.condition,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    &if_expr.then_branch,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                if let Some(else_branch) = &if_expr.else_branch {
                    bind_expr(
                        else_branch,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::While(while_expr, _) => {
                bind_expr(
                    &while_expr.condition,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    &while_expr.body,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
            }
            Expr::For(for_expr, _) => {
                bind_expr(
                    &for_expr.iterable,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                bind_expr(
                    &for_expr.body,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
            }
            Expr::Loop(loop_expr, _) => bind_expr(
                &loop_expr.body,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            Expr::Match(match_expr, _) => {
                bind_expr(
                    &match_expr.scrutinee,
                    eligible_functions,
                    result_bindings,
                    conflicted_bindings,
                );
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        bind_expr(
                            guard,
                            eligible_functions,
                            result_bindings,
                            conflicted_bindings,
                        );
                    }
                    bind_expr(
                        &arm.body,
                        eligible_functions,
                        result_bindings,
                        conflicted_bindings,
                    );
                }
            }
            Expr::Return(Some(expr), _) | Expr::Await(expr, _) => bind_expr(
                expr,
                eligible_functions,
                result_bindings,
                conflicted_bindings,
            ),
            _ => {}
        }
    }

    fn handle_call(
        binding_name: &str,
        args: &[Expr],
        result_bindings: &HashMap<String, String>,
        conflicted_bindings: &HashSet<String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        if conflicted_bindings.contains(binding_name) {
            return;
        }
        let Some(function_name) = result_bindings.get(binding_name) else {
            return;
        };
        let observed: Vec<Option<TypeAnnotation>> =
            args.iter().map(infer_callsite_arg_type).collect();
        let merged = merge_callsite_hint(hints.remove(function_name), observed);
        hints.insert(function_name.clone(), merged);
    }

    fn walk_expr(
        expr: &Expr,
        result_bindings: &HashMap<String, String>,
        conflicted_bindings: &HashSet<String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match expr {
            Expr::FunctionCall { name, args, .. } => {
                handle_call(name, args, result_bindings, conflicted_bindings, hints);
                for arg in args {
                    walk_expr(arg, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::QualifiedFunctionCall { args, .. } => {
                for arg in args {
                    walk_expr(arg, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                walk_expr(receiver, result_bindings, conflicted_bindings, hints);
                for arg in args {
                    walk_expr(arg, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                walk_expr(left, result_bindings, conflicted_bindings, hints);
                walk_expr(right, result_bindings, conflicted_bindings, hints);
            }
            Expr::UnaryOp { operand, .. } | Expr::Reference { expr: operand, .. } => {
                walk_expr(operand, result_bindings, conflicted_bindings, hints)
            }
            Expr::Array(elements, _) => {
                for element in elements {
                    walk_expr(element, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                walk_expr(object, result_bindings, conflicted_bindings, hints);
                walk_expr(index, result_bindings, conflicted_bindings, hints);
            }
            Expr::PropertyAccess { object, .. } => {
                walk_expr(object, result_bindings, conflicted_bindings, hints)
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                walk_expr(condition, result_bindings, conflicted_bindings, hints);
                walk_expr(then_expr, result_bindings, conflicted_bindings, hints);
                if let Some(else_expr) = else_expr {
                    walk_expr(else_expr, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    walk_stmt(stmt, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    walk_block_item(item, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::If(if_expr, _) => {
                walk_expr(
                    &if_expr.condition,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                walk_expr(
                    &if_expr.then_branch,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                if let Some(else_branch) = &if_expr.else_branch {
                    walk_expr(else_branch, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::While(while_expr, _) => {
                walk_expr(
                    &while_expr.condition,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                walk_expr(
                    &while_expr.body,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
            }
            Expr::For(for_expr, _) => {
                walk_expr(
                    &for_expr.iterable,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                walk_expr(&for_expr.body, result_bindings, conflicted_bindings, hints);
            }
            Expr::Loop(loop_expr, _) => {
                walk_expr(&loop_expr.body, result_bindings, conflicted_bindings, hints)
            }
            Expr::Match(match_expr, _) => {
                walk_expr(
                    &match_expr.scrutinee,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        walk_expr(guard, result_bindings, conflicted_bindings, hints);
                    }
                    walk_expr(&arm.body, result_bindings, conflicted_bindings, hints);
                }
            }
            Expr::Return(Some(expr), _) | Expr::Await(expr, _) => {
                walk_expr(expr, result_bindings, conflicted_bindings, hints)
            }
            _ => {}
        }
    }

    fn walk_block_item(
        item: &BlockItem,
        result_bindings: &HashMap<String, String>,
        conflicted_bindings: &HashSet<String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let Some(value) = decl.value.as_ref() {
                    walk_expr(value, result_bindings, conflicted_bindings, hints);
                }
            }
            BlockItem::Assignment(asgn) => {
                walk_expr(&asgn.value, result_bindings, conflicted_bindings, hints)
            }
            BlockItem::Statement(stmt) => {
                walk_stmt(stmt, result_bindings, conflicted_bindings, hints)
            }
            BlockItem::Expression(expr) => {
                walk_expr(expr, result_bindings, conflicted_bindings, hints)
            }
        }
    }

    fn walk_stmt(
        stmt: &Statement,
        result_bindings: &HashMap<String, String>,
        conflicted_bindings: &HashSet<String>,
        hints: &mut HashMap<String, ClosureCallsiteHint>,
    ) {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                walk_expr(expr, result_bindings, conflicted_bindings, hints)
            }
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = decl.value.as_ref() {
                    walk_expr(value, result_bindings, conflicted_bindings, hints);
                }
            }
            Statement::Assignment(asgn, _) => {
                walk_expr(&asgn.value, result_bindings, conflicted_bindings, hints)
            }
            Statement::For(for_stmt, _) => {
                for stmt in &for_stmt.body {
                    walk_stmt(stmt, result_bindings, conflicted_bindings, hints);
                }
            }
            Statement::While(while_stmt, _) => {
                walk_expr(
                    &while_stmt.condition,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                for stmt in &while_stmt.body {
                    walk_stmt(stmt, result_bindings, conflicted_bindings, hints);
                }
            }
            Statement::If(if_stmt, _) => {
                walk_expr(
                    &if_stmt.condition,
                    result_bindings,
                    conflicted_bindings,
                    hints,
                );
                for stmt in &if_stmt.then_body {
                    walk_stmt(stmt, result_bindings, conflicted_bindings, hints);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        walk_stmt(stmt, result_bindings, conflicted_bindings, hints);
                    }
                }
            }
            _ => {}
        }
    }

    let mut eligible_functions = HashSet::new();
    for item in &program.items {
        if let Item::Function(func, _) = item {
            if block_returns_closure(&func.body) {
                eligible_functions.insert(func.name.clone());
            }
        }
    }
    if eligible_functions.is_empty() {
        return HashMap::new();
    }

    let mut result_bindings = HashMap::new();
    let mut conflicted_bindings = HashSet::new();
    for item in &program.items {
        match item {
            Item::Statement(stmt, _) => bind_stmt(
                stmt,
                &eligible_functions,
                &mut result_bindings,
                &mut conflicted_bindings,
            ),
            Item::Expression(expr, _) => bind_expr(
                expr,
                &eligible_functions,
                &mut result_bindings,
                &mut conflicted_bindings,
            ),
            Item::Assignment(asgn, _) => bind_expr(
                &asgn.value,
                &eligible_functions,
                &mut result_bindings,
                &mut conflicted_bindings,
            ),
            Item::VariableDecl(decl, _) => bind_decl(
                decl,
                &eligible_functions,
                &mut result_bindings,
                &mut conflicted_bindings,
            ),
            Item::Function(func, _) => {
                for stmt in &func.body {
                    bind_stmt(
                        stmt,
                        &eligible_functions,
                        &mut result_bindings,
                        &mut conflicted_bindings,
                    );
                }
            }
            _ => {}
        }
    }

    let mut hints = HashMap::new();
    for item in &program.items {
        match item {
            Item::Statement(stmt, _) => {
                walk_stmt(stmt, &result_bindings, &conflicted_bindings, &mut hints)
            }
            Item::Expression(expr, _) => {
                walk_expr(expr, &result_bindings, &conflicted_bindings, &mut hints)
            }
            Item::Assignment(asgn, _) => walk_expr(
                &asgn.value,
                &result_bindings,
                &conflicted_bindings,
                &mut hints,
            ),
            Item::VariableDecl(decl, _) => {
                if let Some(value) = decl.value.as_ref() {
                    walk_expr(value, &result_bindings, &conflicted_bindings, &mut hints);
                }
            }
            Item::Function(func, _) => {
                for stmt in &func.body {
                    walk_stmt(stmt, &result_bindings, &conflicted_bindings, &mut hints);
                }
            }
            _ => {}
        }
    }

    hints
}

/// Strict-typing-sweep (Cluster 2): scan a closure body for binary ops
/// of the form `<param_name> <op> <literal>` (or the symmetric form), and
/// derive a `TypeAnnotation` for `param_name` from the literal's type when
/// the literal has one. This handles the canonical
/// `|x| x + 1` / `|y| y + N` patterns that previously rode on the
/// (now-deleted) Dynamic-emission shim.
///
/// Conservative: returns `None` if the param appears only in untyped
/// contexts, or if the binary op pairs the param with another unknown
/// (e.g. `|x, y| x + y`). The closure body still compiles in those cases
/// — strict-typing simply errors at the offending binary op as before.
pub(crate) fn infer_param_type_from_body(
    param_name: &str,
    body: &[shape_ast::ast::Statement],
) -> Option<TypeAnnotation> {
    use shape_ast::ast::{Literal, Statement};
    fn literal_to_type_ann(lit: &Literal) -> Option<TypeAnnotation> {
        Some(match lit {
            Literal::Int(_) => TypeAnnotation::Basic("int".to_string()),
            Literal::Number(_) => TypeAnnotation::Basic("number".to_string()),
            Literal::Bool(_) => TypeAnnotation::Basic("bool".to_string()),
            Literal::String(_) => TypeAnnotation::Basic("string".to_string()),
            _ => return None,
        })
    }
    fn expr_mentions_name(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::Identifier(n, _) => n == name,
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                expr_mentions_name(left, name) || expr_mentions_name(right, name)
            }
            Expr::UnaryOp { operand, .. } => expr_mentions_name(operand, name),
            Expr::FunctionCall { args, .. } => args.iter().any(|a| expr_mentions_name(a, name)),
            Expr::MethodCall { receiver, args, .. } => {
                expr_mentions_name(receiver, name)
                    || args.iter().any(|a| expr_mentions_name(a, name))
            }
            Expr::Array(elements, _) => elements.iter().any(|e| expr_mentions_name(e, name)),
            Expr::Return(Some(e), _) => expr_mentions_name(e, name),
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .is_some_and(|e| expr_mentions_name(e, name)),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    expr_mentions_name(&assign.value, name)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => stmt_mentions_name(stmt, name),
                shape_ast::ast::BlockItem::Expression(expr) => expr_mentions_name(expr, name),
            }),
            Expr::Assign(assign, _) => {
                expr_mentions_name(&assign.target, name) || expr_mentions_name(&assign.value, name)
            }
            _ => false,
        }
    }
    fn stmt_mentions_name(stmt: &Statement, name: &str) -> bool {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                expr_mentions_name(expr, name)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(|e| expr_mentions_name(e, name)),
            Statement::Assignment(assign, _) => expr_mentions_name(&assign.value, name),
            _ => false,
        }
    }
    fn expr_contains_string_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::String(_), _) => true,
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                expr_contains_string_literal(left) || expr_contains_string_literal(right)
            }
            Expr::UnaryOp { operand, .. } => expr_contains_string_literal(operand),
            Expr::FunctionCall { args, .. } => args.iter().any(expr_contains_string_literal),
            Expr::MethodCall { receiver, args, .. } => {
                expr_contains_string_literal(receiver)
                    || args.iter().any(expr_contains_string_literal)
            }
            Expr::Array(elements, _) => elements.iter().any(expr_contains_string_literal),
            Expr::Return(Some(e), _) => expr_contains_string_literal(e),
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .is_some_and(expr_contains_string_literal),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    expr_contains_string_literal(&assign.value)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => stmt_contains_string_literal(stmt),
                shape_ast::ast::BlockItem::Expression(expr) => expr_contains_string_literal(expr),
            }),
            Expr::Assign(assign, _) => expr_contains_string_literal(&assign.value),
            _ => false,
        }
    }
    fn stmt_contains_string_literal(stmt: &Statement) -> bool {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                expr_contains_string_literal(expr)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(expr_contains_string_literal),
            Statement::Assignment(assign, _) => expr_contains_string_literal(&assign.value),
            _ => false,
        }
    }
    fn scan_expr(name: &str, expr: &Expr) -> Option<TypeAnnotation> {
        match expr {
            Expr::BinaryOp {
                left, op, right, ..
            } => {
                if matches!(op, shape_ast::ast::BinaryOp::Add)
                    && expr_mentions_name(expr, name)
                    && expr_contains_string_literal(expr)
                {
                    return Some(TypeAnnotation::Basic("string".to_string()));
                }
                if let (Expr::Identifier(n, _), Expr::Literal(lit, _)) =
                    (left.as_ref(), right.as_ref())
                {
                    if n == name {
                        if let Some(t) = literal_to_type_ann(lit) {
                            return Some(t);
                        }
                    }
                }
                if let (Expr::Literal(lit, _), Expr::Identifier(n, _)) =
                    (left.as_ref(), right.as_ref())
                {
                    if n == name {
                        if let Some(t) = literal_to_type_ann(lit) {
                            return Some(t);
                        }
                    }
                }
                scan_expr(name, left).or_else(|| scan_expr(name, right))
            }
            Expr::UnaryOp { operand, .. } => scan_expr(name, operand),
            Expr::FunctionCall { args, .. } => args.iter().find_map(|a| scan_expr(name, a)),
            Expr::MethodCall { receiver, args, .. } => {
                scan_expr(name, receiver).or_else(|| args.iter().find_map(|a| scan_expr(name, a)))
            }
            Expr::Array(elements, _) => elements.iter().find_map(|e| scan_expr(name, e)),
            Expr::Return(Some(e), _) => scan_expr(name, e),
            Expr::Block(block, _) => block.items.iter().find_map(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => {
                    decl.value.as_ref().and_then(|e| scan_expr(name, e))
                }
                shape_ast::ast::BlockItem::Assignment(assign) => scan_expr(name, &assign.value),
                shape_ast::ast::BlockItem::Statement(stmt) => scan_stmt(name, stmt),
                shape_ast::ast::BlockItem::Expression(expr) => scan_expr(name, expr),
            }),
            Expr::Assign(assign, _) => scan_expr(name, &assign.value),
            // Match: when the scrutinee is the bare `name`, look at any
            // arm-pattern binding of an identifier and propagate its
            // body/guard usage back to `name`'s type. Conservatively
            // handles the common idiom `match v { x where x > 0 => x }`
            // where `v` and `x` are aliased through pattern binding.
            Expr::Match(match_expr, _) => {
                if let Expr::Identifier(scrutinee_name, _) = match_expr.scrutinee.as_ref() {
                    if scrutinee_name == name {
                        // Look at each arm; if its pattern is a single
                        // identifier `x`, scan the guard + body for
                        // `<x> op <literal>` pairings.
                        for arm in &match_expr.arms {
                            if let shape_ast::ast::Pattern::Identifier {
                                name: bound_name, ..
                            } = &arm.pattern
                            {
                                if let Some(guard) = arm.guard.as_ref() {
                                    if let Some(t) = scan_expr(bound_name, guard) {
                                        return Some(t);
                                    }
                                }
                                if let Some(t) = scan_expr(bound_name, &arm.body) {
                                    return Some(t);
                                }
                            }
                        }
                    }
                }
                // Otherwise just recurse into scrutinee + arms looking
                // for the original name.
                scan_expr(name, &match_expr.scrutinee).or_else(|| {
                    match_expr.arms.iter().find_map(|arm| {
                        arm.guard
                            .as_ref()
                            .and_then(|g| scan_expr(name, g))
                            .or_else(|| scan_expr(name, &arm.body))
                    })
                })
            }
            _ => None,
        }
    }
    fn scan_stmt(name: &str, stmt: &Statement) -> Option<TypeAnnotation> {
        match stmt {
            Statement::Expression(expr, _) => scan_expr(name, expr),
            Statement::Return(Some(e), _) => scan_expr(name, e),
            Statement::VariableDecl(decl, _) => {
                decl.value.as_ref().and_then(|e| scan_expr(name, e))
            }
            Statement::Assignment(asgn, _) => scan_expr(name, &asgn.value),
            _ => None,
        }
    }
    body.iter().find_map(|s| scan_stmt(param_name, s))
}

fn closure_body_requires_numeric_param(
    param_name: &str,
    body: &[shape_ast::ast::Statement],
) -> bool {
    use shape_ast::ast::{BinaryOp, Expr, Statement};

    fn expr_mentions_name(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::Identifier(n, _) => n == name,
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                expr_mentions_name(left, name) || expr_mentions_name(right, name)
            }
            Expr::UnaryOp { operand, .. } => expr_mentions_name(operand, name),
            Expr::FunctionCall { args, .. } => args.iter().any(|a| expr_mentions_name(a, name)),
            Expr::MethodCall { receiver, args, .. } => {
                expr_mentions_name(receiver, name)
                    || args.iter().any(|a| expr_mentions_name(a, name))
            }
            Expr::Array(elements, _) => elements.iter().any(|e| expr_mentions_name(e, name)),
            Expr::Return(Some(e), _) => expr_mentions_name(e, name),
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .is_some_and(|e| expr_mentions_name(e, name)),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    expr_mentions_name(&assign.value, name)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => stmt_mentions_name(stmt, name),
                shape_ast::ast::BlockItem::Expression(expr) => expr_mentions_name(expr, name),
            }),
            Expr::Assign(assign, _) => {
                expr_mentions_name(&assign.target, name) || expr_mentions_name(&assign.value, name)
            }
            Expr::Match(match_expr, _) => {
                expr_mentions_name(&match_expr.scrutinee, name)
                    || match_expr.arms.iter().any(|arm| {
                        arm.guard
                            .as_ref()
                            .is_some_and(|g| expr_mentions_name(g, name))
                            || expr_mentions_name(&arm.body, name)
                    })
            }
            _ => false,
        }
    }

    fn stmt_mentions_name(stmt: &Statement, name: &str) -> bool {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                expr_mentions_name(expr, name)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(|e| expr_mentions_name(e, name)),
            Statement::Assignment(assign, _) => expr_mentions_name(&assign.value, name),
            _ => false,
        }
    }

    fn scan_expr(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::BinaryOp {
                left, op, right, ..
            } => {
                let numeric_op = match op {
                    // `+` is overloaded for string concatenation. When a
                    // param appears in an additive expression that already
                    // carries a string literal, the param is in string
                    // context, not an unproven numeric context.
                    BinaryOp::Add => {
                        !expr_contains_string_literal(left) && !expr_contains_string_literal(right)
                    }
                    BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Pow
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitShl
                    | BinaryOp::BitShr => true,
                    _ => false,
                };
                (numeric_op && (expr_mentions_name(left, name) || expr_mentions_name(right, name)))
                    || scan_expr(left, name)
                    || scan_expr(right, name)
            }
            Expr::UnaryOp { operand, .. } => scan_expr(operand, name),
            Expr::FunctionCall { args, .. } => args.iter().any(|a| scan_expr(a, name)),
            Expr::MethodCall { receiver, args, .. } => {
                scan_expr(receiver, name) || args.iter().any(|a| scan_expr(a, name))
            }
            Expr::Array(elements, _) => elements.iter().any(|e| scan_expr(e, name)),
            Expr::Return(Some(e), _) => scan_expr(e, name),
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => {
                    decl.value.as_ref().is_some_and(|e| scan_expr(e, name))
                }
                shape_ast::ast::BlockItem::Assignment(assign) => scan_expr(&assign.value, name),
                shape_ast::ast::BlockItem::Statement(stmt) => scan_stmt(stmt, name),
                shape_ast::ast::BlockItem::Expression(expr) => scan_expr(expr, name),
            }),
            Expr::Assign(assign, _) => scan_expr(&assign.value, name),
            Expr::Match(match_expr, _) => {
                scan_expr(&match_expr.scrutinee, name)
                    || match_expr.arms.iter().any(|arm| {
                        arm.guard.as_ref().is_some_and(|g| scan_expr(g, name))
                            || scan_expr(&arm.body, name)
                    })
            }
            _ => false,
        }
    }

    fn expr_contains_string_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(shape_ast::ast::Literal::String(_), _) => true,
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                expr_contains_string_literal(left) || expr_contains_string_literal(right)
            }
            Expr::UnaryOp { operand, .. } => expr_contains_string_literal(operand),
            Expr::FunctionCall { args, .. } => args.iter().any(expr_contains_string_literal),
            Expr::MethodCall { receiver, args, .. } => {
                expr_contains_string_literal(receiver)
                    || args.iter().any(expr_contains_string_literal)
            }
            Expr::Array(elements, _) => elements.iter().any(expr_contains_string_literal),
            Expr::Return(Some(e), _) => expr_contains_string_literal(e),
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .is_some_and(expr_contains_string_literal),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    expr_contains_string_literal(&assign.value)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => stmt_contains_string_literal(stmt),
                shape_ast::ast::BlockItem::Expression(expr) => expr_contains_string_literal(expr),
            }),
            Expr::Assign(assign, _) => expr_contains_string_literal(&assign.value),
            Expr::Match(match_expr, _) => {
                expr_contains_string_literal(&match_expr.scrutinee)
                    || match_expr.arms.iter().any(|arm| {
                        arm.guard
                            .as_ref()
                            .is_some_and(|g| expr_contains_string_literal(g))
                            || expr_contains_string_literal(&arm.body)
                    })
            }
            _ => false,
        }
    }

    fn stmt_contains_string_literal(stmt: &Statement) -> bool {
        match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                expr_contains_string_literal(expr)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(expr_contains_string_literal),
            Statement::Assignment(assign, _) => expr_contains_string_literal(&assign.value),
            _ => false,
        }
    }

    fn scan_stmt(stmt: &Statement, name: &str) -> bool {
        match stmt {
            Statement::Expression(expr, _) => scan_expr(expr, name),
            Statement::Return(Some(expr), _) => scan_expr(expr, name),
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(|expr| scan_expr(expr, name)),
            Statement::Assignment(asgn, _) => scan_expr(&asgn.value, name),
            _ => false,
        }
    }

    body.iter().any(|stmt| scan_stmt(stmt, param_name))
}

/// Strict-typing-sweep (Cluster 1): convert a `ConcreteType` (the v2 typed
/// value-representation type) back into an AST `TypeAnnotation` so it can be
/// attached to a synthetic capture parameter. Returning `None` falls back to
/// the no-annotation path (which is fine for opaque types — those captures
/// never participate in typed binary-ops anyway).
///
/// We map the type-name primitives that `tracked_type_name_from_annotation`
/// recognizes plus `Vec<T>` for arrays. Composite/opaque types
/// (Struct/Enum/Closure/Function/Pointer/HashMap with non-trivial inner)
/// return `None` — they don't need typed-op support inside the closure body.
pub(crate) fn concrete_type_to_type_annotation(ct: &ConcreteType) -> Option<TypeAnnotation> {
    match ct {
        ConcreteType::F64 => Some(TypeAnnotation::Basic("number".to_string())),
        ConcreteType::I64 => Some(TypeAnnotation::Basic("int".to_string())),
        ConcreteType::I32 => Some(TypeAnnotation::Basic("i32".to_string())),
        ConcreteType::I16 => Some(TypeAnnotation::Basic("i16".to_string())),
        ConcreteType::I8 => Some(TypeAnnotation::Basic("i8".to_string())),
        ConcreteType::U64 => Some(TypeAnnotation::Basic("u64".to_string())),
        ConcreteType::U32 => Some(TypeAnnotation::Basic("u32".to_string())),
        ConcreteType::U16 => Some(TypeAnnotation::Basic("u16".to_string())),
        ConcreteType::U8 => Some(TypeAnnotation::Basic("u8".to_string())),
        ConcreteType::Bool => Some(TypeAnnotation::Basic("bool".to_string())),
        ConcreteType::String => Some(TypeAnnotation::Basic("string".to_string())),
        ConcreteType::Decimal => Some(TypeAnnotation::Basic("decimal".to_string())),
        ConcreteType::BigInt => Some(TypeAnnotation::Basic("bigint".to_string())),
        ConcreteType::DateTime => Some(TypeAnnotation::Basic("DateTime".to_string())),
        ConcreteType::Array(inner) => {
            // Render as Vec<T> via the Generic form so
            // `tracked_type_name_from_annotation` produces "Vec<int>" /
            // "Vec<number>" — the names the type-tracker keys typed array
            // ops on.
            concrete_type_to_type_annotation(inner).map(|inner_ann| TypeAnnotation::Generic {
                name: TypePath::simple("Vec"),
                args: vec![inner_ann],
            })
        }
        // R3-elemerasure (strict-flip): a struct/enum element type carries its
        // source-level name (`ConcreteType::Struct`/`Enum` → `NamedTypeId`), so
        // render it as a `Reference(name)` annotation. This lets an
        // object-element HOF closure param recover its struct type — e.g.
        // `users.filter(|u| u.score > 85)` gives `u: User`, so the property
        // access `u.score` resolves against the schema instead of surfacing
        // "Cannot infer types for binary operation `Greater`". The name IS the
        // proof (per ADR-006 §2.7.5); an unnamed layout yields `None` (no
        // fabrication — the closure param stays unannotated and the body's
        // property access surfaces a clean error if unresolvable).
        ConcreteType::Struct(named) => named
            .name_str()
            .map(|n| TypeAnnotation::Reference(TypePath::simple(n))),
        ConcreteType::Enum(named) => named
            .name_str()
            .map(|n| TypeAnnotation::Reference(TypePath::simple(n))),
        // Nullable: drop the wrapper — the captured variable is the inner
        // value at the binary-op site if the closure narrows it. No-annotation
        // is safer than a wrong annotation.
        ConcreteType::Option(_) => None,
        // Other composite / opaque types: no useful annotation for the
        // type-tracker. The capture lives as a Pointer-typed slot via the
        // closure layout and does not participate in typed binops.
        _ => None,
    }
}

/// Sweep phase 3c.1: extract a primitive scalar type-name from a
/// runtime `Type`. Mirrors the subset of `numeric_ops::type_display_name`
/// the closure return-type inference cares about.
pub(crate) fn type_display_name_for_closure_inference(
    ty: &shape_runtime::type_system::Type,
) -> String {
    use shape_runtime::type_system::Type;
    match ty {
        Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
        Type::Concrete(TypeAnnotation::Reference(name)) => name.to_string(),
        _ => String::new(),
    }
}

/// U4-2: closure-body return type, SERVED BY THE ENGINE SPAN-TABLE.
///
/// The hand-written `expr_type` mini-inferencer (a FOURTH stringly inference
/// engine, with its own `strip_prefix("Vec<")` re-parse and NO `PropertyAccess`
/// arm → `|p: Emp| { p.salary }` erased to `None`, the live U4 bug) is DELETED.
/// The closure return type now comes from the post-solve span table
/// (`resolved_expr_types`) at the closure body's terminal-expression span — the
/// engine already walks every closure body (U4-0 / P2 made closure-body field
/// reads resolve to their declared field type and survive finalization).
///
/// 1. An explicit `-> T` return annotation is honoured verbatim (it is a
///    declared proof, not an inference result).
/// 2. Otherwise the body's terminal expression span is looked up in the engine
///    table and its resolved `Type` is returned structurally. The name-returning
///    wrapper below exists only for residual tracker/schema-keyed call sites.
///
/// FORWARD BINDER (U4-0 skeptic): a generic `|p: T| p.field` records a
/// `Basic("unknown")` sentinel that survives finalization (structurally a
/// `Type::Concrete`). It is NOT a real type — treat it as un-inferable (return
/// `None`, same as a table miss), so the call site stays a genuine miss and the
/// strict checker surfaces the error rather than serving the type "unknown".
///
/// `caller_arg_type_names` / `enclosing_params` are retained for ABI
/// compatibility with residual call sites but are no longer consulted: the
/// engine resolved the closure body against the closure's declared param types
/// during the whole-program walk, so no caller-context re-seeding is needed
/// here.
pub(crate) fn infer_closure_body_return_type_name_with_caller_context(
    compiler: &mut BytecodeCompiler,
    params: &[shape_ast::ast::FunctionParameter],
    body: &[shape_ast::ast::Statement],
    explicit_return: Option<&TypeAnnotation>,
    enclosing_params: &[shape_ast::ast::FunctionParameter],
    caller_arg_type_names: &[Option<String>],
) -> Option<String> {
    let resolved = infer_closure_body_return_type_with_caller_context(
        compiler,
        params,
        body,
        explicit_return,
        enclosing_params,
        caller_arg_type_names,
    )?;

    // Render the resolved `Type` to the legacy String name shape required by
    // callers that still key schema/tracker state by display name.
    match &resolved {
        shape_runtime::type_system::Type::Concrete(ann) => {
            BytecodeCompiler::tracked_type_name_from_annotation(ann)
        }
        shape_runtime::type_system::Type::Generic { .. } => {
            BytecodeCompiler::inferred_type_to_hint_name(&resolved)
        }
        _ => None,
    }
}

pub(crate) fn infer_closure_body_return_type_with_caller_context(
    compiler: &mut BytecodeCompiler,
    _params: &[shape_ast::ast::FunctionParameter],
    body: &[shape_ast::ast::Statement],
    explicit_return: Option<&TypeAnnotation>,
    _enclosing_params: &[shape_ast::ast::FunctionParameter],
    _caller_arg_type_names: &[Option<String>],
) -> Option<shape_runtime::type_system::Type> {
    // 1. Explicit `-> T` annotation is a declared proof — honour it verbatim.
    if let Some(ann) = explicit_return {
        return Some(shape_runtime::type_system::Type::Concrete(ann.clone()));
    }

    // 2. Engine span-table lookup at the body's terminal-expression span.
    let terminal = closure_body_terminal_expr(body)?;
    let span = shape_ast::ast::Spanned::span(terminal);
    if span.is_dummy() {
        return None;
    }
    let resolved = compiler.resolved_expr_types.get(&span)?;

    // FORWARD BINDER: an unknown-sentinel survives finalization but is NOT a
    // real type — treat it as un-inferable so the call site stays a genuine
    // miss → strict surface-and-stop, never the type literally named "unknown".
    // Parity with the engine (operators.rs:526): match both `Basic("unknown")`
    // and `Reference("unknown")` sentinel shapes via `as_type_name_str`.
    if matches!(
        resolved,
        shape_runtime::type_system::Type::Concrete(ann) if ann.as_type_name_str() == Some("unknown")
    ) {
        return None;
    }

    Some(resolved.clone())
}

/// Find a closure body's terminal expression — the expression whose type IS the
/// closure's return type. Mirrors the terminal-finding logic the deleted
/// mini-inferencer used: the last statement; an explicit `Return(Some(e))` uses
/// `e`; a trailing expression statement uses its expr; a trailing `Block`
/// recurses into the block's last item.
fn closure_body_terminal_expr(body: &[shape_ast::ast::Statement]) -> Option<&Expr> {
    use shape_ast::ast::{BlockItem, Statement};

    fn expr_terminal(expr: &Expr) -> Option<&Expr> {
        match expr {
            Expr::Return(Some(inner), _) => expr_terminal(inner),
            Expr::Block(block, _) => match block.items.last()? {
                BlockItem::Expression(e) => expr_terminal(e),
                BlockItem::Statement(s) => stmt_terminal(s),
                _ => None,
            },
            other => Some(other),
        }
    }

    fn stmt_terminal(stmt: &Statement) -> Option<&Expr> {
        match stmt {
            Statement::Expression(e, _) => expr_terminal(e),
            Statement::Return(Some(e), _) => expr_terminal(e),
            _ => None,
        }
    }

    stmt_terminal(body.last()?)
}

impl BytecodeCompiler {
    pub(crate) fn callable_return_hint_name_for_expr(&self, expr: &Expr) -> Option<String> {
        callable_selection_arity(expr)?;
        let function_name = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .map(|function| function.name.clone())?;
        if function_name.starts_with("__closure_") {
            return None;
        }
        self.closure_callsite_param_hints
            .contains_key(&function_name)
            .then_some(function_name)
    }

    /// Compile a function expression (closure)
    ///
    /// `closure_span` is the span of the `||`/`|args|` expression itself
    /// — used by Session 1's Rust-move move-after-capture diagnostic to
    /// point at the capturing closure that consumed a `let mut` binding.
    pub(super) fn compile_expr_closure(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
        closure_span: Span,
    ) -> Result<()> {
        let closure_name = format!("__closure_{}", self.closure_counter);
        self.closure_counter += 1;

        let proto_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, mut mutated_captures) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));
        let captured_var_set: BTreeSet<String> = captured_vars.iter().cloned().collect();
        mutated_captures.extend(collect_static_mut_self_container_captures(
            self,
            body,
            &captured_var_set,
        ));

        // Inside function bodies the MIR solver detects reference-capture errors
        // via `closure_capture_loans` facts, producing `ReferenceEscapeIntoClosure`.
        // For top-level code (no MIR), we still reject at the front-end.
        // Exception: inferred-ref locals (params passed by reference for performance)
        // are owned values and CAN be captured — the value is dereferenced at capture time.
        if self.current_function.is_none() {
            for captured in &captured_vars {
                if let Some(local_idx) = self.resolve_local(captured) {
                    let escapes_direct_borrow = self.ref_locals.contains(&local_idx)
                        && !self.inferred_ref_locals.contains(&local_idx);
                    let escapes_reference_value = self.reference_value_locals.contains(&local_idx);
                    if escapes_direct_borrow || escapes_reference_value {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                                captured
                            ),
                            location: None,
                        });
                    }
                }

                if let Some(scoped_name) = self.resolve_scoped_module_binding_name(captured)
                    && let Some(&binding_idx) = self.module_bindings.get(&scoped_name)
                    && self.reference_value_module_bindings.contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                            captured
                        ),
                        location: None,
                    });
                }
            }
        }

        // BUG1 — reject assignment to an immutable (`let`) outer binding
        // from inside the closure body. The environment analyzer marks
        // the binding in `mutated_captures` when the closure writes to
        // it; if the outer binding's ownership class is `OwnedImmutable`
        // (the `let` form), the write violates Shape's immutability
        // rules. Without this check the compiler still lowers a
        // `MakeClosure` whose capture layout mismatches the legacy
        // SharedCell path, producing the runtime-only crash
        // `MakeClosure for function N has no registered ClosureLayout`.
        // The diagnostic uses code `B0005` — the same code used for other
        // immutability/move violations across closure boundaries — and
        // suggests both `let mut` (local mutation) and `var` (shareable
        // mutation through closure captures) to match CLAUDE.md guidance.
        for captured in &captured_vars {
            if !mutated_captures.contains(captured) {
                continue;
            }
            let ownership = self
                .binding_semantics_for_name(captured)
                .map(|(_, _, sem)| sem.ownership_class);
            if !matches!(ownership, Some(BindingOwnershipClass::OwnedImmutable)) {
                continue;
            }
            let is_local_slot = self.resolve_local(captured).is_some();
            let is_module_binding_slot = !is_local_slot
                && (self.resolve_scoped_module_binding_name(captured).is_some()
                    || self.module_bindings.contains_key(captured));
            if !is_local_slot && !is_module_binding_slot {
                continue;
            }
            return Err(ShapeError::SemanticError {
                message: format!(
                    "[B0005] cannot assign to immutable binding '{captured}' captured by \
                     closure; use `let mut {captured}` for local mutation or `var {captured}` \
                     to allow shared mutation through closures"
                ),
                location: Some(self.span_to_source_location(closure_span)),
            });
        }

        // Build per-capture cell-access flags (aligned with captured_vars order).
        // Historically this vector was named "mutable_captures", but in the
        // strict layout it means "not a leading immutable param": the body
        // must access the capture through `frame.upvalues` because the slot is
        // `CaptureKind::OwnedMutable` or `CaptureKind::Shared`.
        //
        // A capture needs that path if the closure itself mutates it, if a
        // previous closure already promoted the source to a SharedCell, or if
        // the source binding is `var` (`BindingOwnershipClass::Flexible`).
        // The last case matters for sibling closures: a read-only closure over
        // a `var` must observe the same shared cell as a mutating sibling, not
        // snapshot the current cell contents at construction time.
        let mutable_flags: Vec<bool> = captured_vars
            .iter()
            .map(|name| {
                let is_local_slot = self.resolve_local(name).is_some();
                let is_module_binding_slot = !is_local_slot
                    && (self.resolve_scoped_module_binding_name(name).is_some()
                        || self.module_bindings.contains_key(name));
                let ownership = self
                    .binding_semantics_for_name(name)
                    .map(|(_, _, sem)| sem.ownership_class);
                let is_flexible_capture =
                    matches!(ownership, Some(BindingOwnershipClass::Flexible))
                        && (is_local_slot || is_module_binding_slot);

                mutated_captures.contains(name)
                    || self.boxed_locals.contains(name)
                    || self.shared_locals.contains(name)
                    || self.shared_module_binding_contains(name)
                    || is_flexible_capture
            })
            .collect();

        // Build closure parameters: only immutable captures become leading params.
        // Mutable captures are accessed via LoadClosure/StoreClosure opcodes.
        //
        // Strict-typing-sweep (Cluster 1): synthesize a `type_annotation` for each
        // capture from its resolved upstream `ConcreteType`. Without this the
        // capture-param falls into the "no annotation" branch in
        // `compile_function_body` (line ~1182) and ends up in `param_locals` with
        // no type info — which then makes binary-ops on the capture inside the
        // closure body fail with "Cannot infer types for binary operation".
        let mut closure_params = Vec::with_capacity(captured_vars.len() + params.len());
        for name in &captured_vars {
            let capture_ct = self.resolve_capture_concrete_type(name);
            let type_annotation = concrete_type_to_type_annotation(&capture_ct);
            closure_params.push(shape_ast::ast::FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation,
                default_value: None,
            });
        }

        // Strict-typing-sweep (Cluster 3): consume bidirectional inference
        // hints for the user-portion params. The outer HOF dispatch site
        // populates `pending_closure_param_types` with one Option<TypeAnnotation>
        // per user param when the receiver type implies an arg type
        // (`arr.map(|x| …)` with `arr: Array<int>` → `x: int`). User params
        // with their own explicit annotation always win.
        let user_param_hints = self.pending_closure_param_types.take();

        // Wave 1a PART A: bidirectional inference from the let-binding's call
        // sites. When this closure literal is the initializer of `let f = …`
        // and `f` is invoked directly elsewhere (`f(2, 3)`), the whole-program
        // pre-pass recorded the per-arg argument types. We seed each
        // still-unannotated user param from those types here. `Conflict`
        // (incompatible call sites, or a shadowed name) yields no hints — the
        // closure keeps its existing rejection (strict-typing: no silent pick).
        // `pending_variable_name` holds the binding name during the let-init
        // compile (set in `Statement::VariableDecl` / `Item::VariableDecl`).
        let callsite_param_hints: Option<Vec<Option<TypeAnnotation>>> = self
            .pending_variable_name
            .as_ref()
            .or(self.pending_callable_hint_name.as_ref())
            .and_then(|name| self.closure_callsite_param_hints.get(name))
            .and_then(|hint| match hint {
                ClosureCallsiteHint::Types(types) => Some(types.clone()),
                ClosureCallsiteHint::Conflict => None,
            });
        let binding_fact_param_hints: Option<Vec<Option<TypeAnnotation>>> = self
            .pending_variable_span
            .and_then(|span| self.inference_facts.binding_type(span))
            .and_then(|ty| function_type_param_hints(ty, params.len()));
        let returned_callsite_param_hints: Option<Vec<Option<TypeAnnotation>>> = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .and_then(|function| {
                self.returned_closure_callsite_param_hints
                    .get(&function.name)
            })
            .and_then(|hint| match hint {
                ClosureCallsiteHint::Types(types) => Some(types.clone()),
                ClosureCallsiteHint::Conflict => None,
            });
        let returned_closure_param_hints: Option<Vec<Option<TypeAnnotation>>> = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .and_then(|function| self.inference_facts.function_signature(&function.name))
            .and_then(|signature| match signature.canonicalize() {
                Type::Function { returns, .. } => {
                    function_type_param_hints(returns.as_ref(), params.len())
                }
                _ => None,
            });

        // Strict-typing-sweep (Cluster 2): closure-body param inference.
        // For closures bound to a `let` and called via the local (or
        // synthesized inside a generic body where const-args have been
        // substituted to literals), we don't have an HOF-style call-site
        // hint. Infer each unannotated user param's type by scanning the
        // body for binary ops `<param> op <literal>` and pulling the
        // literal's type. This is the same conservative heuristic that
        // closure compilation has always relied on for `|x| x + 1`-shaped
        // bodies, just made first-class instead of riding on the deleted
        // *Dynamic-emission shim.
        for (idx, user_param) in params.iter().enumerate() {
            let mut p = user_param.clone();
            if p.type_annotation.is_none() {
                // 1. HOF call-site hint wins first.
                if let Some(hints) = user_param_hints.as_ref() {
                    if let Some(Some(ann)) = hints.get(idx) {
                        p.type_annotation = Some(ann.clone());
                    }
                }
                // 1b. Wave 1a PART A: let-binding direct-call-site hint.
                //     `let f = |a, b| a + b; f(2, 3)` flows `a: int, b: int`
                //     back into the params. Applied only to params still
                //     unannotated after the HOF hint.
                if p.type_annotation.is_none() {
                    if let Some(hints) = callsite_param_hints.as_ref() {
                        if let Some(Some(ann)) = hints.get(idx) {
                            p.type_annotation = Some(ann.clone());
                        }
                    }
                }
                // 1c. Function-typed facts from the binder span and from an
                //     enclosing function's inferred function return. These are
                //     solver-produced compile-time facts, so they rescue
                //     stored closures and factory-returned closures without
                //     runtime probing or numeric defaulting.
                if p.type_annotation.is_none() {
                    if let Some(hints) = binding_fact_param_hints.as_ref() {
                        if let Some(Some(ann)) = hints.get(idx) {
                            p.type_annotation = Some(ann.clone());
                        }
                    }
                }
                if p.type_annotation.is_none() {
                    if let Some(hints) = returned_callsite_param_hints.as_ref() {
                        if let Some(Some(ann)) = hints.get(idx) {
                            p.type_annotation = Some(ann.clone());
                        }
                    }
                }
                if p.type_annotation.is_none() {
                    if let Some(hints) = returned_closure_param_hints.as_ref() {
                        if let Some(Some(ann)) = hints.get(idx) {
                            p.type_annotation = Some(ann.clone());
                        }
                    }
                }
                // 2. Body-level literal-pairing heuristic. Pulls type
                //    info from any binary op pairing the param with a
                //    typed literal OR with a captured/outer-scope
                //    identifier whose type is known.
                if p.type_annotation.is_none() {
                    if let Some(name) = p.pattern.as_identifier() {
                        if let Some(ann) = infer_param_type_from_body(name, body) {
                            p.type_annotation = Some(ann);
                        } else if let Some(ann) =
                            self.infer_param_type_from_body_with_outer_idents(name, body)
                        {
                            p.type_annotation = Some(ann);
                        }
                        if p.type_annotation.is_none()
                            && closure_body_requires_numeric_param(name, body)
                        {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "cannot infer the numeric type of closure parameter `{}` at compile time; annotate the parameter as `int` or `number`",
                                    name
                                ),
                                location: Some(self.span_to_source_location(closure_span)),
                            });
                        }
                    }
                }
            }
            closure_params.push(p);
        }

        let closure_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: closure_params,
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let user_pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        let mut closure_pass_modes =
            vec![crate::compiler::ParamPassMode::ByValue; captured_vars.len()];
        closure_pass_modes.extend(user_pass_modes);
        let ref_params: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_reference())
            .collect();
        let ref_mutates: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_exclusive())
            .collect();
        self.inferred_param_pass_modes
            .insert(closure_name.clone(), closure_pass_modes);

        // Phase A: mint a ClosureTypeId keyed on the capture signature.
        //
        // Resolves each captured name to a `ConcreteType` via the monomorphizer
        // helpers; unresolved captures fall back to `Pointer(Void)` (opaque
        // 8-byte slot, conservatively treated as a heap-refcounted pointer by
        // the layout's `heap_capture_mask`). This records layout metadata in
        // `closure_registry` that Phase C consumes to extend the monomorphization
        // cache key. Emission is unchanged.
        // C2 Bucket-3 carrier-stamp fix: record which captures are invoked
        // as callees in this closure body so `resolve_capture_concrete_type`
        // (used by both `mint_closure_type_id` below and the kinds-aware
        // re-intern further down) classifies an unannotated callable capture
        // as `ConcreteType::Function` (→ `Ptr(HeapKind::Closure)`) rather
        // than the `Pointer(Void)` → `NativeView` "unknown" sentinel. Set
        // for the duration of this closure's capture-type resolution and
        // cleared after the re-intern block.
        let saved_callee_captures = std::mem::replace(
            &mut self.current_closure_callee_captures,
            Self::collect_callee_identifier_names(body),
        );

        let closure_type_id = self.mint_closure_type_id(&captured_vars);

        // Phase F: mint a FunctionTypeId for the callable signature. This is
        // the `Function<A, R>` identity — the signature omits captures and
        // covers only the parameters the caller supplies plus the return.
        //
        // Phase F keeps signature resolution conservative: param / return
        // types that lack compile-time resolution fall back to `Void`. The
        // ID is still globally unique per structural signature (driven by
        // the registry's intern), so `CallFunctionIndirect` can pick a
        // Cranelift call signature once signature inference lands. Two
        // closures with structurally identical callable shapes share a
        // `FunctionTypeId` even when their capture layouts (and hence
        // `ClosureTypeId`s) differ — this is exactly what `Array<Function<
        // (int) -> int>>` relies on for polymorphic dispatch.
        let function_type_id = self.mint_function_type_id_for_params(params);

        let func_idx = self.program.functions.len();
        self.program.functions.push(Function {
            name: closure_name.clone(),
            arity: closure_def.params.len() as u16,
            param_names: closure_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: captured_vars.len() as u16,
            is_async: false,
            ref_params,
            ref_mutates,
            mutable_captures: mutable_flags.clone(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        });

        // Record closure function_id for MIR back-patching (ClosurePlaceholder → Function)
        self.closure_function_ids
            .push((closure_name.clone(), func_idx as u16));
        // Phase A: record the closure's ClosureTypeId against its function index.
        self.closure_type_ids
            .push((func_idx as u16, closure_type_id));
        // Phase F: record the closure's FunctionTypeId alongside the capture
        // layout id. One entry per closure literal, same ordering as
        // `closure_type_ids`.
        self.function_type_ids
            .push((func_idx as u16, function_type_id));

        // Track A.1C — derive the `CaptureKind` for each capture based on
        // the source binding's declared form AND whether the closure body
        // actually needs cell-backed capture access.
        //
        // Binding form (when cell-backed) → CaptureKind:
        //   `let mut x = ...`   (OwnedMutable source)   → CaptureKind::OwnedMutable
        //   `var x = ...`       (Flexible source)       → CaptureKind::Shared
        //
        // Everything else (including read-only captures of `let mut`, and all
        // captures of `let` / function parameters) → `CaptureKind::Immutable`.
        // Read-only `var` captures are intentionally excluded from the
        // snapshot path: `var` is the shared/smart binding form, so sibling
        // closures must observe the same `SharedCell` state.
        //
        // Note (A.1C partial): this metadata rides on the layout's
        // `capture_kinds` field only. The mutable-mask bits on the layout
        // remain zero in this commit — see the design note on
        // `build_closure_function_layouts`. The interpreter's
        // `op_make_closure` still routes mutable-capture closures through
        // the legacy `HeapValue::Closure` + SharedCell path because the
        // compiler has not yet been rewired to emit the A.1B
        // `Load/StoreOwnedMutableCapture` / `Load/StoreSharedCapture`
        // opcodes in closure bodies, and outer-scope reads of promoted
        // `let mut` / `var` bindings still flow through `LoadClosure` +
        // `HeapValue::SharedCell` auto-deref. Full routing is the A.1C
        // residual.
        use shape_value::v2::closure_layout::CaptureKind;
        let capture_kinds: Vec<CaptureKind> = captured_vars
            .iter()
            .enumerate()
            .map(|(i, name)| {
                // Only cell-access captures need indirection. Read-only
                // non-`var` captures are snapshot-by-value and stay Immutable
                // regardless of the source binding's ownership class. This
                // keeps function-parameter captures (default `OwnedMutable`
                // per `binding_semantics_for_param`) on the Immutable path
                // when the closure doesn't write through them.
                if !mutable_flags.get(i).copied().unwrap_or(false) {
                    return CaptureKind::Immutable;
                }
                // Track A.1C.2 (locals) + A.1C.3 (module bindings): any
                // mutable `var` capture routes through
                // `CaptureKind::Shared`, whether the outer slot is a
                // local or a module binding. Both paths allocate an
                // `Arc<parking_lot::Mutex<u64>>` and install its
                // `Arc::into_raw` pointer into the closure's Ptr slot;
                // `op_make_closure` bumps the strong count. The compiler
                // emits different *outer-scope* opcodes for local vs
                // module-binding promotion (`AllocSharedLocal` vs
                // `AllocSharedModuleBinding`), but the closure-side
                // machinery is the same.
                let is_local_slot = self.resolve_local(name).is_some();
                let is_module_binding_slot = !is_local_slot
                    && (self.resolve_scoped_module_binding_name(name).is_some()
                        || self.module_bindings.contains_key(name));
                let ownership = self
                    .binding_semantics_for_name(name)
                    .map(|(_, _, sem)| sem.ownership_class);
                match ownership {
                    // Track A.1C.2b: `let mut` captures whose outer slot
                    // is a local flow through the A.1B OwnedMutable
                    // Raw path. For module-binding `let mut` (top-level
                    // `let mut sum = 0` in REPL-style eval compiles to
                    // a module binding), there is no move-into-closure
                    // semantics — the binding is program-lifetime. Fall
                    // through to the Shared pipeline so mutations from
                    // the closure propagate to the outer slot, matching
                    // the pre-A.1C.3 legacy SharedCell semantics.
                    Some(BindingOwnershipClass::OwnedMutable) if is_local_slot => {
                        CaptureKind::OwnedMutable
                    }
                    Some(BindingOwnershipClass::OwnedMutable) if is_module_binding_slot => {
                        CaptureKind::Shared
                    }
                    Some(BindingOwnershipClass::OwnedMutable) => CaptureKind::Immutable,
                    Some(BindingOwnershipClass::Flexible)
                        if is_local_slot || is_module_binding_slot =>
                    {
                        CaptureKind::Shared
                    }
                    Some(BindingOwnershipClass::Flexible) => CaptureKind::Immutable,
                    // Track A.1C.2 / A.1C.3: semantics lookup can return
                    // `None` when a prior closure's `compile_function`
                    // wiped the outer function's type-tracker local
                    // semantics. Fall back to persistent witnesses
                    // populated by the previous classification pass:
                    //   - `shared_locals` / `shared_module_bindings`
                    //     for `var` captures.
                    //   - `owned_mutable_locals` for `let mut` local
                    //     captures (A.1C.3: without this witness, a
                    //     second closure capturing a different local
                    //     would reclassify to `Immutable`, nulling the
                    //     layout's OwnedMutable mask and tripping the
                    //     `op_make_closure` layout-mismatch guard).
                    _ if is_local_slot && self.shared_locals.contains(name) => CaptureKind::Shared,
                    _ if is_local_slot && self.owned_mutable_locals.contains(name) => {
                        CaptureKind::OwnedMutable
                    }
                    _ if is_module_binding_slot && self.shared_module_binding_contains(name) => {
                        CaptureKind::Shared
                    }
                    // A.1C.3: module-binding captures with no resolved
                    // ownership semantics (e.g. imported functions used
                    // as callable values, top-level `let` without `mut`
                    // — unreachable here since `mutable_flags[i]` is
                    // true) also go through Shared when the closure
                    // mutates them. `mutable_flags[i]` is already known
                    // true at this point (early return above).
                    _ if is_module_binding_slot => CaptureKind::Shared,
                    _ => CaptureKind::Immutable,
                }
            })
            .collect();
        // Track A.1C.3: record persistent witnesses for each classified
        // capture so sibling closures (after the type-tracker has been
        // wiped by `compile_function`) reclassify the same way rather
        // than falling back to `Immutable`.
        for (i, name) in captured_vars.iter().enumerate() {
            match capture_kinds[i] {
                CaptureKind::OwnedMutable if self.resolve_local(name).is_some() => {
                    self.owned_mutable_locals.insert(name.clone());
                }
                _ => {}
            }
        }
        self.closure_capture_kinds
            .push((func_idx as u16, capture_kinds.clone()));

        // Track A.1C.2: if any capture is non-Immutable, re-intern the
        // closure_type_id under the kinds-aware registry key so two
        // closures with identical types but different kinds get distinct
        // `ClosureTypeId`s. When all captures are Immutable, the original
        // types-only intern already returned the canonical id — skip.
        if capture_kinds
            .iter()
            .any(|k| !matches!(k, CaptureKind::Immutable))
        {
            let mut capture_types: Vec<ConcreteType> = Vec::with_capacity(captured_vars.len());
            for name in &captured_vars {
                capture_types.push(self.resolve_capture_concrete_type(name));
            }
            let kinds_id = self
                .closure_registry
                .intern_with_kinds(capture_types, capture_kinds.clone());
            // Overwrite the last-pushed `closure_type_ids` entry for this
            // function with the kinds-aware id. The Immutable entry
            // produced by `mint_closure_type_id` (which ignores kinds)
            // remains in the registry for all-immutable closures.
            if let Some(last) = self.closure_type_ids.last_mut() {
                debug_assert_eq!(last.0, func_idx as u16);
                last.1 = kinds_id;
            }
            let _ = closure_type_id; // the kinds-aware id supersedes it.
        }

        // Capture-type resolution for this closure is complete; restore the
        // enclosing closure's callee-capture set (closures nest).
        self.current_closure_callee_captures = saved_callee_captures;

        // Track A.1C.2b — enforce `let mut` escape rejection (§4.3).
        //
        // `let mut` bindings captured by an escaping closure are a
        // compile error: `let mut` is a unique-owner form, and moving
        // it into a heap closure that outlives the surrounding frame
        // would leak the owner out of its original scope. The compiler
        // rejects this with B0003 and asks the user to promote the
        // source to `var` (shared) or restructure. Non-escaping
        // closures (the common case) are fine — the `let mut` binding
        // is moved by value into a single closure at make-closure time
        // and accessed inside the body via `LoadOwnedMutableCapture` /
        // `StoreOwnedMutableCapture` (A.1B).
        //
        // The heap-promotion signal is `emit_make_closure_heap_next`.
        let closure_is_escaping = self.emit_make_closure_heap_next;
        for (i, name) in captured_vars.iter().enumerate() {
            if !mutable_flags.get(i).copied().unwrap_or(false) {
                continue;
            }
            let local_idx = self.resolve_local(name);
            let plan_class = local_idx.and_then(|idx| self.mir_storage_class_for_slot(idx));
            let ownership = self
                .binding_semantics_for_name(name)
                .map(|(_, _, sem)| sem.ownership_class);

            if matches!(ownership, Some(BindingOwnershipClass::OwnedMutable))
                && !matches!(
                    plan_class,
                    Some(BindingStorageClass::LocalMutablePtr)
                        | Some(BindingStorageClass::Reference)
                        | Some(BindingStorageClass::Direct)
                        | Some(BindingStorageClass::Deferred)
                        | None,
                )
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0003] mutable binding '{}' cannot be captured by an escaping closure; \
                         promote the source to `var` or restructure to keep the closure local",
                        name
                    ),
                    location: None,
                });
            }
        }

        // Set up the per-kind closure-body emission maps. During body
        // compilation:
        //   * `mutable_closure_captures` → legacy `LoadClosure` /
        //     `StoreClosure` (module-binding `var` captures and any
        //     residual capture whose outer slot could not be migrated
        //     to A.1B's Raw path).
        //   * `owned_mutable_closure_captures` → A.1B's
        //     `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture`
        //     for `let mut` captures (outer slot is moved by value into
        //     the closure at make-closure time; closure owns the
        //     `Box::into_raw(Box::new(initial))` pointer).
        //   * `shared_closure_captures` → A.1B's `LoadSharedCapture` /
        //     `StoreSharedCapture` for `var` (local-slot) captures
        //     previously promoted via `AllocSharedLocal`.
        let saved_mutable_captures = std::mem::take(&mut self.mutable_closure_captures);
        let saved_shared_captures = std::mem::take(&mut self.shared_closure_captures);
        let saved_owned_mutable_captures = std::mem::take(&mut self.owned_mutable_closure_captures);
        let saved_owned_mutable_capture_inner_kinds =
            std::mem::take(&mut self.owned_mutable_capture_inner_kinds);
        let saved_shared_capture_inner_kinds = std::mem::take(&mut self.shared_capture_inner_kinds);
        let _ = closure_is_escaping;
        for (i, name) in captured_vars.iter().enumerate() {
            if mutable_flags.get(i).copied().unwrap_or(false) {
                self.mutable_closure_captures.insert(name.clone(), i as u16);
                let kind = capture_kinds
                    .get(i)
                    .copied()
                    .unwrap_or(CaptureKind::Immutable);
                // Track A.1C.2 + A.1C.3: Shared (var) captures — whether
                // the outer slot is a local or a module binding — route
                // through the A.1B Load/StoreSharedCapture opcodes
                // inside the closure body. The closure-side machinery
                // is identical; only the outer-scope promotion opcodes
                // differ between locals and module bindings.
                if matches!(kind, CaptureKind::Shared) {
                    self.shared_closure_captures.insert(name.clone(), i as u16);
                    // A2-refined / task #17: record the cell's interior
                    // `FieldKind` so the closure body's Shared read/write
                    // emit sites can dispatch to the typed Wave D.2
                    // opcodes (codes 0x156-0x16B), mirroring the
                    // OwnedMutable population a few lines below. The
                    // inner kind is derived from the captured binding's
                    // resolved `ConcreteType`. Falls back to `Ptr` when
                    // the type isn't statically resolved.
                    let inner_kind = self.resolve_capture_concrete_type(name).to_field_kind();
                    self.shared_capture_inner_kinds
                        .insert(name.clone(), inner_kind);
                }
                // Track A.1C.2b: OwnedMutable (let mut) captures route
                // through the A.1B Load/StoreOwnedMutableCapture
                // opcodes. Gate on `resolve_local` — only locals can be
                // captured OwnedMutable (module bindings have program-
                // lifetime and don't admit move semantics); for module-
                // binding sources the capture was reclassified to
                // `Immutable` upstream.
                if matches!(kind, CaptureKind::OwnedMutable) && self.resolve_local(name).is_some() {
                    self.owned_mutable_closure_captures
                        .insert(name.clone(), i as u16);
                    // Wave E: record the cell's interior `FieldKind` so the
                    // closure body's read/write emit sites can dispatch to
                    // the typed Wave D.1 opcodes (codes 0x140-0x155). The
                    // inner kind is derived from the captured binding's
                    // resolved `ConcreteType` at this construction site —
                    // identical to the type used for `op_make_closure`'s
                    // `alloc_owned_mutable_<kind>` selection. Falls back to
                    // `Ptr` when the type isn't statically resolved
                    // (matches `concrete_type_for_expr`'s default for
                    // unresolved heap-typed captures).
                    let inner_kind = self.resolve_capture_concrete_type(name).to_field_kind();
                    self.owned_mutable_capture_inner_kinds
                        .insert(name.clone(), inner_kind);
                }
            }
        }

        // Jump-over is now emitted unconditionally inside
        // `compile_function_body`, which patches its own jump at the end of
        // the body. Emitting another jump here would double-jump and the
        // closure's entry_point (post-the-outer-jump) would point at the
        // inner jump, which then skips the body entirely. Don't.
        let saved_closure_ids = self.closure_function_ids.clone();
        let saved_pending_callable_hint_name = self.pending_callable_hint_name.take();
        let closure_destructure_facts = closure_array_binding_facts(&closure_def.params);
        let closure_binding_types = closure_binding_fact_types_by_name(&closure_destructure_facts);
        let closure_expr_type_overrides =
            collect_closure_expr_type_overrides(&closure_def.body, &closure_binding_types);
        let saved_resolved_expr_types = if closure_expr_type_overrides.is_empty() {
            None
        } else {
            let saved = self.resolved_expr_types.clone();
            self.resolved_expr_types.extend(closure_expr_type_overrides);
            Some(saved)
        };
        let saved_inference_facts = inference_facts_with_closure_binding_facts(
            &self.inference_facts,
            closure_destructure_facts,
        )
        .map(|facts| std::mem::replace(&mut self.inference_facts, facts));
        let compile_result = self.compile_function(&closure_def);
        if let Some(saved) = saved_inference_facts {
            self.inference_facts = saved;
        }
        if let Some(saved) = saved_resolved_expr_types {
            self.resolved_expr_types = saved;
        }
        self.pending_callable_hint_name = saved_pending_callable_hint_name;
        self.closure_function_ids = saved_closure_ids;
        compile_result?;

        // Restore mutable_closure_captures
        self.mutable_closure_captures = saved_mutable_captures;
        self.shared_closure_captures = saved_shared_captures;
        self.owned_mutable_closure_captures = saved_owned_mutable_captures;
        self.owned_mutable_capture_inner_kinds = saved_owned_mutable_capture_inner_kinds;
        self.shared_capture_inner_kinds = saved_shared_capture_inner_kinds;

        // Capture boxing decisions
        // ────────────────────────
        // The storage planner assigns each binding a BindingStorageClass that
        // determines whether the variable needs heap indirection:
        //
        //   Direct     → LoadLocal / StoreLocal (no indirection needed)
        //   Deferred   → plan not yet resolved; fall back to legacy boxing
        //   UniqueHeap → legacy cell wrapping + SharedCell.
        //                Future: unique Box without RwLock overhead.
        //   SharedCow  → legacy cell wrapping + SharedCell.
        //                Future: COW wrapper.
        //   Reference  → DerefLoad / DerefStore (already handled above)
        //
        // We emit the legacy cell-wrapping opcode when the storage plan says
        // the binding needs heap indirection (UniqueHeap, SharedCow, Direct,
        // or Deferred). Only Reference bindings skip boxing — they are
        // handled separately by the escape check above. In the future, the
        // planner may introduce a dedicated "no-sharing" class to skip
        // boxing for Direct bindings.
        for (i, captured) in captured_vars.iter().enumerate() {
            if matches!(
                self.binding_semantics_for_name(captured),
                Some((_, _, semantics))
                    if semantics.ownership_class == BindingOwnershipClass::Flexible
            ) {
                let storage = if mutable_flags.get(i).copied().unwrap_or(false) {
                    BindingStorageClass::SharedCow
                } else {
                    BindingStorageClass::UniqueHeap
                };
                self.promote_flexible_binding_storage_for_name(captured, storage);
            }
            if mutable_flags.get(i).copied().unwrap_or(false) {
                // Consult the storage plan to decide whether boxing is needed.
                // Currently, Direct and Deferred bindings are both boxed for
                // mutable captures because the storage plan runs before closure
                // compilation and these are the default states. Reference
                // bindings are already handled by the escape check above, so
                // the only class that could skip boxing is one where the
                // planner explicitly marks "no sharing needed" — a future
                // optimization.
                // Consult the MIR storage plan first (authoritative when available),
                // then fall back to type-tracker binding semantics.
                let mir_plan_class = self
                    .resolve_local(captured)
                    .and_then(|idx| self.mir_storage_class_for_slot(idx));
                let should_box = if let Some(plan_class) = mir_plan_class {
                    // MIR plan is authoritative: box when UniqueHeap/SharedCow,
                    // skip for Reference (handled above), box for Direct/Deferred
                    // since mutable capture needs heap indirection.
                    !matches!(plan_class, BindingStorageClass::Reference)
                } else if let Some((_, _, semantics)) = self.binding_semantics_for_name(captured) {
                    // Fallback to type-tracker semantics
                    !matches!(semantics.storage_class, BindingStorageClass::Reference)
                } else {
                    true // no plan available, use legacy behavior (always box)
                };

                if should_box {
                    // Mutable capture: promote the outer binding so the
                    // closure and its enclosing scope observe the same
                    // mutable state, then push the value (OwnedMutable) or
                    // pointer (Shared) the enclosing `MakeClosure` needs
                    // to install into the closure's capture slot.
                    //
                    // Dispatch by `capture_kinds[i]`:
                    //   * `Shared` (`var` binding captured mutably) →
                    //     Track A.1C.2 path. For local slots: emit
                    //     `LoadLocal + AllocSharedLocal + LoadLocal` to
                    //     promote the slot into `Arc<SharedCell>` and
                    //     push the pointer bits; add the binding to
                    //     `shared_locals` so every outer-scope read /
                    //     write / scope-exit goes through the new
                    //     opcodes. For module bindings keep the legacy
                    //     `BoxModuleBinding` path — A.1C.1's opcodes
                    //     cover only local slots; module bindings retire
                    //     with A.1C.3.
                    //   * `OwnedMutable` (`let mut`) → Track A.1C.2b
                    //     path. Push the outer slot's plain value with
                    //     `LoadLocal`; `op_make_closure` will see the
                    //     `owned_mutable_capture_mask` bit for this
                    //     index and call
                    //     `Box::into_raw(Box::new(initial))`. The closure
                    //     body emits
                    //     `Load/StoreOwnedMutableCapture` (A.1B) to read
                    //     /write through the box pointer. No SharedCell,
                    //     no Arc, no lock.
                    //   * Other fallbacks (module-binding `var` etc.) →
                    //     legacy cell-wrapping / `BoxModuleBinding` path.
                    //     A.1C.3 retires these alongside the
                    //     `HeapValue::Closure` fallback producer.
                    self.set_binding_storage_class_for_name(
                        captured,
                        BindingStorageClass::SharedCow,
                    );
                    let kind = capture_kinds
                        .get(i)
                        .copied()
                        .unwrap_or(CaptureKind::Immutable);
                    let is_shared_local_slot = matches!(kind, CaptureKind::Shared)
                        && self.resolve_local(captured).is_some();
                    let is_owned_mutable = matches!(kind, CaptureKind::OwnedMutable);
                    let shared_module_binding_scoped_name =
                        if matches!(kind, CaptureKind::Shared) && !is_shared_local_slot {
                            self.resolve_scoped_module_binding_name(captured)
                                .or_else(|| {
                                    if self.module_bindings.contains_key(captured) {
                                        Some(captured.clone())
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };
                    if is_shared_local_slot {
                        let local_idx = self
                            .resolve_local(captured)
                            .expect("checked is_shared_local_slot");
                        if !self.shared_locals.contains(captured) {
                            // First promotion: push current value, alloc
                            // the Arc cell, then push the pointer bits.
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                            self.emit(Instruction::new(
                                OpCode::AllocSharedLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                            self.shared_locals.insert(captured.clone());
                            if let Some(scope) = self.shared_drop_locals.last_mut() {
                                scope.push(local_idx);
                            }
                        }
                        // Push the *pointer bits* of the (possibly just-
                        // allocated) shared cell. op_make_closure will
                        // `Arc::increment_strong_count` for each Shared
                        // capture before installing it in the closure.
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if is_owned_mutable && let Some(local_idx) = self.resolve_local(captured)
                    {
                        // Track A.1C.2b: `let mut` outer slot is captured
                        // by move. Push the current value — op_make_closure
                        // sees the `owned_mutable_capture_mask` bit and
                        // allocates `Box::into_raw(Box::new(bits))` into
                        // the Ptr slot. No cell wrapping, no SharedCell.
                        //
                        // Session 1 — Rust-move semantics: record the
                        // binding as "moved into closure at closure_span"
                        // so subsequent outer reads / writes fail at
                        // compile time with a use-after-move diagnostic.
                        // The `captured_let_mut_moved` map is consulted
                        // in `compile_expr_identifier` (load path) and
                        // `compile_expr_assign` (store path).
                        self.captured_let_mut_moved
                            .insert(captured.clone(), closure_span);
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if let Some(scoped_name) = shared_module_binding_scoped_name {
                        // Track A.1C.3: Shared module-binding var
                        // capture. Mirrors the Shared local-slot path
                        // above with module-binding addressing:
                        //   First promotion: `LoadModuleBinding` +
                        //     `AllocSharedModuleBinding` promotes the
                        //     module-binding slot to raw Arc pointer
                        //     bits.
                        //   Then: `LoadModuleBinding` pushes those raw
                        //     pointer bits for `op_make_closure` to
                        //     `Arc::increment_strong_count` on.
                        // `LoadModuleBinding`'s auto-deref for legacy
                        // SharedCell is retired in this same commit —
                        // the bits pushed here are raw pointer bits,
                        // not a tagged SharedCell carrier, so
                        // `LoadModuleBinding` passes them through
                        // unmodified.
                        let mb_idx = self.get_or_create_module_binding(&scoped_name);
                        if !self.shared_module_bindings.contains(&scoped_name) {
                            self.emit(Instruction::new(
                                OpCode::LoadModuleBinding,
                                Some(Operand::ModuleBinding(mb_idx)),
                            ));
                            self.emit(Instruction::new(
                                OpCode::AllocSharedModuleBinding,
                                Some(Operand::ModuleBinding(mb_idx)),
                            ));
                            self.shared_module_bindings.insert(scoped_name);
                        }
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(mb_idx)),
                        ));
                    } else {
                        // Last resort fallback — just load the value.
                        // Reached when the capture is Immutable (e.g.
                        // OwnedMutable that resolved to a module
                        // binding and was reclassified). A plain load
                        // is correct: op_make_closure will store the
                        // raw bits directly into the capture slot as
                        // an Immutable capture.
                        let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                        self.compile_expr(&temp)?;
                    }
                } else {
                    // Storage plan says Direct — no boxing needed, just load the value.
                    let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                    self.compile_expr(&temp)?;
                }
            } else {
                let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                self.compile_expr(&temp)?;
                // Phase V1.2C/D — Site A: closure capture of a
                // uniquely-owned value into an *escaping* closure.
                // If the outer slot is classified as `UniqueHeap`
                // (Box-backed, owned — see Phase 4 / `PromoteToOwned`)
                // and the closure escapes the current scope, the
                // captured value must transition to an Arc-shared
                // encoding so the closure can outlive the owning
                // binding. `PromoteToShared` converts the top-of-stack
                // Box into an Arc in place without bumping a refcount.
                // No-op on inline scalars and already-Arc values, so
                // emitting it here is correctness-safe; gating on
                // `UniqueHeap` simply avoids the unnecessary opcode.
                //
                // Non-escaping closures share the caller's scope by
                // construction — the Box stays unique for the closure's
                // lifetime and the promotion is unnecessary.
                if closure_is_escaping && crate::compiler::helpers::promote_to_shared_enabled() {
                    if let Some(local_idx) = self.resolve_local(captured) {
                        // Mirror V1.1C's `slot_is_heap_backed_owned`:
                        // `UniqueHeap` is the canonical owned-heap class,
                        // but `Direct` + non-scalar storage hint also
                        // indicates a Box-backed slot (strings, arrays,
                        // hashmaps, typed objects) handed to the slot
                        // by the Phase 4 `PromoteToOwned` emission —
                        // those need the same Box→Arc transition when
                        // they escape into a closure.
                        if self.slot_is_heap_backed_owned(local_idx) {
                            self.emit(Instruction::simple(OpCode::PromoteToShared));
                        }
                    }
                }
            }
        }

        // Phase F: when the compiler has been told to emit the heap-ABI
        // form for this closure (e.g. by an outer expression that knows the
        // closure escapes — the most common driver is return-of-closure and
        // store-into-array patterns), tag the `MakeClosure` operand with
        // `escapes: true`. Phase H5 merged the former `MakeClosureHeap`
        // opcode into `MakeClosure`; the JIT reads `escapes` from the
        // operand variant (compile-time constant — no memory load on the
        // dispatch fast path).
        //
        // The `emit_make_closure_heap_next` flag is a single-shot hook: the
        // caller sets it before `compile_expr_closure` runs and the
        // closure lowerer consumes it at emission time. This keeps the
        // decision close to the escape signal without threading a second
        // parameter through the closure-compilation API.
        let escapes = std::mem::take(&mut self.emit_make_closure_heap_next);
        let fid = shape_value::FunctionId(func_idx as u16);
        let operand = if escapes {
            Operand::ClosureAlloc { fid, escapes: true }
        } else {
            Operand::Function(fid)
        };
        self.emit(Instruction::new(OpCode::MakeClosure, Some(operand)));
        // Closures don't produce TypedObjects
        self.last_expr_schema = None;
        // A closure value is a heap-tagged Arc<HeapValue::ClosureRaw>, NOT
        // a numeric type. Clear any numeric/type-info signal that leaked
        // from the closure body's last evaluated expression so the
        // surrounding `let inc = || { ... }` doesn't fall into the
        // typed-I64/F64 emission path (`emit_store_local_for_hint` →
        // `StoreLocalI64`). Routing closure bindings to the polymorphic
        // legacy `StoreLocal`/`LoadLocal` is required because the typed
        // local handlers don't perform Arc retain/release on their
        // 8-byte slot, leading to a use-after-free of the closure block
        // when the binding is loaded for a call (see #104 / #95).
        self.last_expr_type_info = None;
        Ok(())
    }

    /// Read-only access to the compiler's closure registry.
    /// Populated by each closure literal during lowering (Phase A).
    pub fn closure_registry(&self) -> &shape_value::v2::closure_layout::ClosureRegistry {
        &self.closure_registry
    }

    /// `(function_id, ClosureTypeId)` pairs, one per closure literal lowered
    /// during compilation. Phase C consumes this to key the monomorphization
    /// cache by closure layout.
    pub fn closure_type_ids(&self) -> &[(u16, ClosureTypeId)] {
        &self.closure_type_ids
    }

    /// Read-only access to the compiler's function-type registry.
    /// Populated per closure literal during lowering (Phase F).
    pub fn function_type_registry(
        &self,
    ) -> &shape_value::v2::function_type_registry::FunctionTypeRegistry {
        &self.function_type_registry
    }

    /// `(function_id, FunctionTypeId)` pairs, one per closure literal.
    /// Phase F uses this to pick a Cranelift `call_indirect` signature for
    /// polymorphic `Function<A, R>` dispatch.
    pub fn function_type_ids(&self) -> &[(u16, shape_value::v2::concrete_type::FunctionTypeId)] {
        &self.function_type_ids
    }

    /// Mint a `ClosureTypeId` for a closure literal by resolving each capture
    /// name to a `ConcreteType` and interning the resulting signature in
    /// `closure_registry` (Phase A).
    ///
    /// Unresolved captures fall back to `Pointer(Void)` — an opaque 8-byte
    /// slot that the layout treats as heap-refcounted. This keeps semantics
    /// conservative (no missed Drop glue) while Phase B/C/D grow the
    /// resolution coverage.
    pub(crate) fn mint_closure_type_id(&mut self, captured_vars: &[String]) -> ClosureTypeId {
        let mut capture_types: Vec<ConcreteType> = Vec::with_capacity(captured_vars.len());
        for name in captured_vars {
            capture_types.push(self.resolve_capture_concrete_type(name));
        }
        self.closure_registry.intern(capture_types)
    }

    /// Resolve a captured variable's `ConcreteType` for closure-layout kind
    /// tracking (ADR-006 §2.7.8 / Q10).
    ///
    /// The capture's `NativeKind` (derived from this `ConcreteType` via
    /// `native_kind_from_concrete_type`) drives the refcount discipline in
    /// `clone_with_kind` / `drop_with_kind` AND the callee-kind check in
    /// `call_value_immediate_nb`. Resolution order:
    ///
    /// 1. `concrete_type_for_expr` — the side-table / element-type path
    ///    (covers annotated params, array/map element types, recorded
    ///    let-binding ConcreteTypes).
    /// 2. Compile-time inference/type facts via `infer_expr_type`. An
    ///    **unannotated function/closure param** (e.g. `g` in
    ///    `fn wrap(g) { |x| g(x) }`) has no side-table entry, but inference
    ///    resolves it to `Type::Function`. Mapping it to
    ///    `ConcreteType::Function` here is the carrier-stamp fix: without
    ///    it the capture falls through to the `Pointer(Void)` "unknown"
    ///    sentinel below, which `native_kind_from_concrete_type` maps to
    ///    `Ptr(HeapKind::NativeView)` — a wrong-carrier label that both
    ///    mis-dispatches the refcount (`Arc<NativeViewData>` vs the
    ///    closure's `Arc<HeapValue>`) AND makes the returned closure
    ///    un-callable (the `call_value_immediate_nb` callee match rejects
    ///    `Ptr(NativeView)`).
    /// 3. `Pointer(Void)` — the conservative "opaque heap-refcounted slot"
    ///    fallback for genuinely-unresolved captures. NOTE: this is still
    ///    mapped to `NativeView` downstream; it is correct only for non-
    ///    callable opaque captures and is preserved as-is (no scope creep).
    fn resolve_capture_concrete_type(&mut self, name: &str) -> ConcreteType {
        let ident = Expr::Identifier(name.to_string(), Span::DUMMY);
        if let Some(ct) = concrete_type_for_expr(self, &ident) {
            return ct;
        }
        // U4-6: when the regular slot concrete tables miss, recover the
        // binding's finalized type from compile-time inference facts through the
        // binder span recorded at slot creation. This replaces the former
        // collection-constructor side-table and keeps the capture stamp tied to
        // the canonical inference pass.
        if let Some(ct) =
            crate::compiler::monomorphization::type_resolution::binding_fact_capture_type(
                self, name,
            )
        {
            return ct;
        }
        // Carrier-stamp fix (C2 Bucket-3): a capture invoked as a callee
        // (`g(...)`) inside the closure body is a closure/function value.
        // Stamp it `ConcreteType::Function` so the capture (and the
        // returned closure that holds it) carries `Ptr(HeapKind::Closure)`
        // — `native_kind_from_concrete_type` maps `Function` → Closure.
        // Without this it falls through to the `Pointer(Void)` "unknown"
        // sentinel which downstream becomes `Ptr(HeapKind::NativeView)`,
        // a wrong-carrier label that mis-dispatches the refcount
        // (`Arc<NativeViewData>` vs the closure's `Arc<HeapValue>`) and is
        // rejected by the `call_value_immediate_nb` callee match.
        if self.current_closure_callee_captures.contains(name) {
            return ConcreteType::Function(shape_value::v2::concrete_type::FunctionTypeId(0));
        }
        ConcreteType::Pointer(Box::new(ConcreteType::Void))
    }

    /// Collect identifier names invoked in callee position (`name(...)`)
    /// anywhere inside `body`. Used to classify unannotated callable
    /// closure captures (see `resolve_capture_concrete_type`).
    pub(crate) fn collect_callee_identifier_names(
        body: &[shape_ast::ast::Statement],
    ) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for stmt in body {
            collect_callee_names_in_stmt(stmt, &mut out);
        }
        out
    }

    /// Phase F — mint a `FunctionTypeId` for a closure literal's callable
    /// signature (parameters + return type).
    ///
    /// Captures are intentionally excluded: `FunctionTypeId` identifies the
    /// cross-value `Function<A, R>` shape, not the capture layout. Two
    /// closures with the same signature but different captures share a
    /// `FunctionTypeId` — this is the whole point of the `Array<Function<
    /// (int) -> int>>` dispatch pattern.
    ///
    /// Resolution of per-param concrete types from type annotations is
    /// kept conservative in Phase F: unannotated or unresolved params
    /// resolve to `ConcreteType::Void`. This is safe because the registry
    /// keys structurally and two closures with identical (annotated) param
    /// shapes still share an id; Phase G/H will tighten resolution once
    /// bidirectional inference is wired through.
    pub(crate) fn mint_function_type_id_for_params(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
    ) -> shape_value::v2::concrete_type::FunctionTypeId {
        use shape_value::v2::concrete_type::ConcreteType as CT;
        use shape_value::v2::function_type_registry::FunctionSignature;

        let param_types: Vec<CT> = params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_ref()
                    .and_then(Self::concrete_type_for_annotation_static)
                    .unwrap_or(CT::Void)
            })
            .collect();
        let ret = CT::Void;
        self.function_type_registry
            .intern(FunctionSignature::new(param_types, ret))
    }

    /// Extract a `ConcreteType` from a `TypeAnnotation` without consulting
    /// the compiler's type-inference machinery. Lightweight, conservative
    /// mapping for the Phase F `FunctionTypeId` registry.
    fn concrete_type_for_annotation_static(
        annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Option<shape_value::v2::concrete_type::ConcreteType> {
        use shape_ast::ast::TypeAnnotation;
        use shape_value::v2::concrete_type::ConcreteType as CT;
        match annotation {
            TypeAnnotation::Basic(name) => match name.as_str() {
                "int" | "i64" => Some(CT::I64),
                "i32" => Some(CT::I32),
                "i16" => Some(CT::I16),
                "i8" => Some(CT::I8),
                "u64" => Some(CT::U64),
                "u32" => Some(CT::U32),
                "u16" => Some(CT::U16),
                "u8" => Some(CT::U8),
                "number" | "f64" => Some(CT::F64),
                "bool" => Some(CT::Bool),
                "string" => Some(CT::String),
                "void" | "unit" => Some(CT::Void),
                "decimal" => Some(CT::Decimal),
                "bigint" => Some(CT::BigInt),
                "DateTime" | "datetime" => Some(CT::DateTime),
                _ => None,
            },
            TypeAnnotation::Array(inner) => {
                Self::concrete_type_for_annotation_static(inner).map(|t| CT::Array(Box::new(t)))
            }
            TypeAnnotation::Reference(path) => {
                let name = path.as_str();
                match name {
                    "int" | "i64" => Some(CT::I64),
                    "number" | "f64" => Some(CT::F64),
                    "bool" => Some(CT::Bool),
                    "string" => Some(CT::String),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Phase C — peek a closure literal's capture signature and mint (or
    /// reuse) a [`ClosureTypeId`] WITHOUT lowering the closure to bytecode
    /// and WITHOUT pushing to `closure_type_ids`.
    ///
    /// The resolver calls this during `try_monomorphize_method_call` to key
    /// the monomorphization cache on the closure's layout. At emission time
    /// the usual `compile_expr_closure` path runs as normal — because the
    /// registry's `intern` is idempotent, both calls return the same
    /// `ClosureTypeId`. The split responsibility (gotcha option **(a)** in
    /// the Phase C plan) is:
    ///
    ///   - Resolver → peek + intern layout id only.
    ///   - `compile_expr_closure` → intern layout id (no-op second time) AND
    ///     push `(func_id, type_id)` into `closure_type_ids`.
    ///
    /// This keeps `closure_type_ids` free of duplicates while letting the
    /// resolver see the id early.
    pub(crate) fn mint_closure_type_id_peek(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> ClosureTypeId {
        // Run the same capture analysis as `compile_expr_closure`, but only
        // for the purpose of reading capture names off of the AST.
        let proto_def = FunctionDef {
            name: "__peek_closure__".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, _mutated) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));

        // Mirror `compile_expr_closure`'s callee-capture classification so
        // the peeked `ClosureTypeId` matches the real one (a callable
        // capture stamps `ConcreteType::Function`, not `Pointer(Void)`).
        let saved_callee_captures = std::mem::replace(
            &mut self.current_closure_callee_captures,
            Self::collect_callee_identifier_names(body),
        );
        let id = self.mint_closure_type_id(&captured_vars);
        self.current_closure_callee_captures = saved_callee_captures;
        id
    }

    /// Strict-typing-sweep (Cluster 2 extension): same body scan as the
    /// free `infer_param_type_from_body` helper but uses the compiler's
    /// type tracker to resolve identifier operands against outer-scope
    /// bindings. Catches `|x| x + n` where `n` is a captured int local.
    pub(crate) fn infer_param_type_from_body_with_outer_idents(
        &self,
        param_name: &str,
        body: &[shape_ast::ast::Statement],
    ) -> Option<TypeAnnotation> {
        use shape_ast::ast::Statement;
        fn scan_expr(
            compiler: &BytecodeCompiler,
            name: &str,
            expr: &Expr,
        ) -> Option<TypeAnnotation> {
            match expr {
                Expr::BinaryOp { left, right, .. } => {
                    let pair_match = if let Expr::Identifier(n, _) = left.as_ref() {
                        if n == name {
                            static_expr_type_ann(compiler, right, name)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(ann) = pair_match {
                        return Some(ann);
                    }
                    let pair_match = if let Expr::Identifier(n, _) = right.as_ref() {
                        if n == name {
                            static_expr_type_ann(compiler, left, name)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(ann) = pair_match {
                        return Some(ann);
                    }
                    scan_expr(compiler, name, left).or_else(|| scan_expr(compiler, name, right))
                }
                Expr::UnaryOp { operand, .. } => scan_expr(compiler, name, operand),
                Expr::FunctionCall { args, .. } => {
                    args.iter().find_map(|a| scan_expr(compiler, name, a))
                }
                Expr::MethodCall { receiver, args, .. } => scan_expr(compiler, name, receiver)
                    .or_else(|| args.iter().find_map(|a| scan_expr(compiler, name, a))),
                Expr::Array(elements, _) => {
                    elements.iter().find_map(|e| scan_expr(compiler, name, e))
                }
                Expr::Return(Some(e), _) => scan_expr(compiler, name, e),
                Expr::Block(block, _) => block.items.iter().find_map(|item| match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                        .value
                        .as_ref()
                        .and_then(|e| scan_expr(compiler, name, e)),
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        scan_expr(compiler, name, &assign.value)
                    }
                    shape_ast::ast::BlockItem::Statement(stmt) => scan_stmt(compiler, name, stmt),
                    shape_ast::ast::BlockItem::Expression(expr) => scan_expr(compiler, name, expr),
                }),
                Expr::Assign(assign, _) => scan_expr(compiler, name, &assign.value),
                _ => None,
            }
        }
        fn scan_stmt(
            compiler: &BytecodeCompiler,
            name: &str,
            stmt: &Statement,
        ) -> Option<TypeAnnotation> {
            match stmt {
                Statement::Expression(expr, _) => scan_expr(compiler, name, expr),
                Statement::Return(Some(e), _) => scan_expr(compiler, name, e),
                Statement::VariableDecl(decl, _) => decl
                    .value
                    .as_ref()
                    .and_then(|e| scan_expr(compiler, name, e)),
                Statement::Assignment(asgn, _) => scan_expr(compiler, name, &asgn.value),
                _ => None,
            }
        }
        /// Resolve an arbitrary expression to a `TypeAnnotation` when it's
        /// statically known and does not mention the parameter being inferred.
        /// This keeps inference bidirectional but static: `base + offset + x`
        /// can seed `x` from the proven type of `base + offset`, while
        /// self-dependent shapes like `x + (x + n)` stay unproven.
        fn static_expr_type_ann(
            compiler: &BytecodeCompiler,
            expr: &Expr,
            param_name: &str,
        ) -> Option<TypeAnnotation> {
            if expr_mentions_name(expr, param_name) {
                return None;
            }
            if let Some(ct) = concrete_type_for_expr(compiler, expr) {
                return concrete_type_to_type_annotation(&ct);
            }
            let other_name = match expr {
                Expr::Identifier(n, _) => n,
                _ => return None,
            };
            let ident_expr = Expr::Identifier(other_name.clone(), Span::DUMMY);
            if let Some(ct) = concrete_type_for_expr(compiler, &ident_expr) {
                return concrete_type_to_type_annotation(&ct);
            }
            let hint = if let Some(local_idx) = compiler.resolve_local(other_name) {
                compiler.type_tracker.get_local_storage_hint(local_idx)
            } else {
                let scoped = compiler
                    .resolve_scoped_module_binding_name(other_name)
                    .unwrap_or_else(|| other_name.to_string());
                compiler
                    .module_bindings
                    .get(&scoped)
                    .or_else(|| compiler.module_bindings.get(other_name))
                    .and_then(|&idx| compiler.type_tracker.get_binding_type(idx))
                    .and_then(|info| info.storage_hint)
            }?;
            storage_hint_to_type_annotation(hint)
        }

        fn expr_mentions_name(expr: &Expr, name: &str) -> bool {
            match expr {
                Expr::Identifier(n, _) => n == name,
                Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                    expr_mentions_name(left, name) || expr_mentions_name(right, name)
                }
                Expr::UnaryOp { operand, .. } | Expr::Reference { expr: operand, .. } => {
                    expr_mentions_name(operand, name)
                }
                Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => {
                    args.iter().any(|a| expr_mentions_name(a, name))
                }
                Expr::MethodCall { receiver, args, .. } => {
                    expr_mentions_name(receiver, name)
                        || args.iter().any(|a| expr_mentions_name(a, name))
                }
                Expr::Array(elements, _) => elements.iter().any(|e| expr_mentions_name(e, name)),
                Expr::IndexAccess { object, index, .. } => {
                    expr_mentions_name(object, name) || expr_mentions_name(index, name)
                }
                Expr::PropertyAccess { object, .. } => expr_mentions_name(object, name),
                Expr::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                    ..
                } => {
                    expr_mentions_name(condition, name)
                        || expr_mentions_name(then_expr, name)
                        || else_expr
                            .as_ref()
                            .is_some_and(|expr| expr_mentions_name(expr, name))
                }
                Expr::Return(Some(expr), _) | Expr::Await(expr, _) => {
                    expr_mentions_name(expr, name)
                }
                Expr::Block(block, _) => block.items.iter().any(|item| match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                        .value
                        .as_ref()
                        .is_some_and(|expr| expr_mentions_name(expr, name)),
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        expr_mentions_name(&assign.value, name)
                    }
                    shape_ast::ast::BlockItem::Statement(stmt) => stmt_mentions_name(stmt, name),
                    shape_ast::ast::BlockItem::Expression(expr) => expr_mentions_name(expr, name),
                }),
                Expr::Assign(assign, _) => {
                    expr_mentions_name(&assign.target, name)
                        || expr_mentions_name(&assign.value, name)
                }
                Expr::If(if_expr, _) => {
                    expr_mentions_name(&if_expr.condition, name)
                        || expr_mentions_name(&if_expr.then_branch, name)
                        || if_expr
                            .else_branch
                            .as_ref()
                            .is_some_and(|expr| expr_mentions_name(expr, name))
                }
                Expr::While(while_expr, _) => {
                    expr_mentions_name(&while_expr.condition, name)
                        || expr_mentions_name(&while_expr.body, name)
                }
                Expr::For(for_expr, _) => {
                    expr_mentions_name(&for_expr.iterable, name)
                        || expr_mentions_name(&for_expr.body, name)
                }
                Expr::Loop(loop_expr, _) => expr_mentions_name(&loop_expr.body, name),
                Expr::Match(match_expr, _) => {
                    expr_mentions_name(&match_expr.scrutinee, name)
                        || match_expr.arms.iter().any(|arm| {
                            arm.guard
                                .as_ref()
                                .is_some_and(|guard| expr_mentions_name(guard, name))
                                || expr_mentions_name(&arm.body, name)
                        })
                }
                _ => false,
            }
        }

        fn stmt_mentions_name(stmt: &Statement, name: &str) -> bool {
            match stmt {
                Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                    expr_mentions_name(expr, name)
                }
                Statement::VariableDecl(decl, _) => decl
                    .value
                    .as_ref()
                    .is_some_and(|expr| expr_mentions_name(expr, name)),
                Statement::Assignment(assign, _) => expr_mentions_name(&assign.value, name),
                Statement::If(if_stmt, _) => {
                    expr_mentions_name(&if_stmt.condition, name)
                        || if_stmt
                            .then_body
                            .iter()
                            .any(|stmt| stmt_mentions_name(stmt, name))
                        || if_stmt.else_body.as_ref().is_some_and(|body| {
                            body.iter().any(|stmt| stmt_mentions_name(stmt, name))
                        })
                }
                Statement::While(while_stmt, _) => {
                    expr_mentions_name(&while_stmt.condition, name)
                        || while_stmt
                            .body
                            .iter()
                            .any(|stmt| stmt_mentions_name(stmt, name))
                }
                Statement::For(for_stmt, _) => {
                    for_init_mentions_name(&for_stmt.init, name)
                        || for_stmt
                            .body
                            .iter()
                            .any(|stmt| stmt_mentions_name(stmt, name))
                }
                _ => false,
            }
        }

        fn for_init_mentions_name(init: &shape_ast::ast::ForInit, name: &str) -> bool {
            match init {
                shape_ast::ast::ForInit::ForIn { iter, .. } => expr_mentions_name(iter, name),
                shape_ast::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    stmt_mentions_name(init, name)
                        || expr_mentions_name(condition, name)
                        || expr_mentions_name(update, name)
                }
            }
        }

        fn storage_hint_to_type_annotation(
            hint: crate::type_tracking::StorageHint,
        ) -> Option<TypeAnnotation> {
            use shape_value::NativeKind;

            let name = match hint {
                NativeKind::Float64 => "number",
                NativeKind::Int64 => "int",
                NativeKind::Int32 => "i32",
                NativeKind::Int16 => "i16",
                NativeKind::Int8 => "i8",
                NativeKind::UInt64 => "u64",
                NativeKind::UInt32 => "u32",
                NativeKind::UInt16 => "u16",
                NativeKind::UInt8 => "u8",
                NativeKind::Bool => "bool",
                NativeKind::String => "string",
                _ => return None,
            };
            Some(TypeAnnotation::Basic(name.to_string()))
        }
        body.iter().find_map(|s| scan_stmt(self, param_name, s))
    }
}

/// Recursively collect callee-position identifier names from a statement.
/// Best-effort: handles the common closure-body shapes. Any expression form
/// not explicitly recursed into is conservatively ignored (falls back to the
/// pre-fix `Pointer(Void)` capture classification — no regression).
fn collect_callee_names_in_stmt(
    stmt: &shape_ast::ast::Statement,
    out: &mut std::collections::BTreeSet<String>,
) {
    use shape_ast::ast::Statement;
    match stmt {
        Statement::Return(Some(e), _) => collect_callee_names_in_expr(e, out),
        Statement::Return(None, _) => {}
        Statement::Expression(e, _) => collect_callee_names_in_expr(e, out),
        Statement::VariableDecl(decl, _) => {
            if let Some(e) = &decl.value {
                collect_callee_names_in_expr(e, out);
            }
        }
        Statement::Assignment(assign, _) => collect_callee_names_in_expr(&assign.value, out),
        Statement::If(if_stmt, _) => {
            collect_callee_names_in_expr(&if_stmt.condition, out);
            for s in &if_stmt.then_body {
                collect_callee_names_in_stmt(s, out);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    collect_callee_names_in_stmt(s, out);
                }
            }
        }
        Statement::While(w, _) => {
            collect_callee_names_in_expr(&w.condition, out);
            for s in &w.body {
                collect_callee_names_in_stmt(s, out);
            }
        }
        Statement::For(f, _) => {
            for s in &f.body {
                collect_callee_names_in_stmt(s, out);
            }
        }
        _ => {}
    }
}

/// Recursively collect callee-position identifier names from an expression.
/// The load-bearing arm is `Expr::FunctionCall { name, .. }` — a bare
/// `name(...)` where `name` is a captured variable means that capture holds
/// a callable (closure / function) value. Compound expressions recurse so
/// `g(x) + h(y)` etc. are covered; nested `FunctionExpr` bodies recurse too
/// since an inner closure may invoke a variable captured from the outer one.
fn collect_callee_names_in_expr(
    expr: &shape_ast::ast::Expr,
    out: &mut std::collections::BTreeSet<String>,
) {
    use shape_ast::ast::Expr;
    match expr {
        Expr::FunctionCall { name, args, .. } => {
            out.insert(name.clone());
            for a in args {
                // R1 named-fn-as-value capture fix: an identifier passed
                // directly as a call argument may be a function value
                // forwarded onward (`apply(g, y)` where `g` is an
                // unannotated capture holding a named-fn-id). Classify it
                // as a callable candidate so `resolve_capture_concrete_type`
                // stamps the capture slot `Ptr(HeapKind::Closure)` and the
                // `op_make_closure` carrier reconciliation can materialize a
                // real closure carrier. Captures with a known non-function
                // concrete type resolve via `concrete_type_for_expr` first
                // (it is consulted before this set), so this does not
                // mis-classify typed captures; a residual mismatch is caught
                // by `op_make_closure`'s runtime guard as a clean error.
                if let Expr::Identifier(arg_name, _) = a {
                    out.insert(arg_name.clone());
                }
                collect_callee_names_in_expr(a, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_callee_names_in_expr(receiver, out);
            for a in args {
                if let Expr::Identifier(arg_name, _) = a {
                    out.insert(arg_name.clone());
                }
                collect_callee_names_in_expr(a, out);
            }
        }
        Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
            collect_callee_names_in_expr(left, out);
            collect_callee_names_in_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_callee_names_in_expr(operand, out),
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            collect_callee_names_in_expr(condition, out);
            collect_callee_names_in_expr(then_expr, out);
            if let Some(e) = else_expr {
                collect_callee_names_in_expr(e, out);
            }
        }
        Expr::Array(elems, _) => {
            for e in elems {
                collect_callee_names_in_expr(e, out);
            }
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            collect_callee_names_in_expr(object, out);
            collect_callee_names_in_expr(index, out);
            if let Some(e) = end_index {
                collect_callee_names_in_expr(e, out);
            }
        }
        Expr::PropertyAccess { object, .. } => collect_callee_names_in_expr(object, out),
        Expr::Return(Some(e), _) | Expr::Break(Some(e), _) => collect_callee_names_in_expr(e, out),
        Expr::TryOperator(e, _) | Expr::Await(e, _) | Expr::Spread(e, _) => {
            collect_callee_names_in_expr(e, out)
        }
        Expr::FunctionExpr { body, .. } => {
            for s in body {
                collect_callee_names_in_stmt(s, out);
            }
        }
        Expr::Block(block, _) => {
            use shape_ast::ast::expr_helpers::BlockItem;
            for item in &block.items {
                match item {
                    BlockItem::VariableDecl(decl) => {
                        if let Some(e) = &decl.value {
                            collect_callee_names_in_expr(e, out);
                        }
                    }
                    BlockItem::Assignment(assign) => {
                        collect_callee_names_in_expr(&assign.value, out)
                    }
                    BlockItem::Statement(s) => collect_callee_names_in_stmt(s, out),
                    BlockItem::Expression(e) => collect_callee_names_in_expr(e, out),
                }
            }
        }
        _ => {}
    }
}

// Wave-β C-expressions: the closures `tests` module (closure spec phase D
// + Track A.1B/A.1C migration coverage, ~2100 lines) was deleted along
// with this sweep. Every test asserted via the deleted carrier
// (`run_program_top_level` returned the carrier; assertions called
// scalar accessors that no longer exist; the H3 single-variant upvalue
// guard constructed `Upvalue::new(...)` with the deleted carrier).
// The opcode-emission predicates (e.g. `any_escaping_make_closure`,
// `is_any_load_owned_mutable_capture`) survive structurally inside
// `crate::compiler::helpers` / `crate::bytecode::Operand` and can be
// rebuilt cheaply once the phase-2c carrier shape (ADR-006 §2.4) and
// the test harness sweep on `crate::test_utils::eval` land. The Track
// A.1C.3 module-binding `var` capture coverage in particular needs to
// be restored alongside the closure-cell parallel-kind invariant
// (ADR-006 §2.7.8 / Q10).
