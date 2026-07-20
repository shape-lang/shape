//! Expression compilation
//!
//! This module contains the main expression compilation logic, organized by expression type.

use shape_ast::ast::{Expr, Span};
use shape_ast::error::{Result, ShapeError};

use super::{
    BorrowMode, BytecodeCompiler, ExprReferenceResult, ExprResultMode, HygienicRole,
};
use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_runtime::type_schema::FieldType;

/// U4-4: return TYPE of a compiler-internal builtin / `__intrinsic_*` math
/// function, by bare name. These builtins' bodies are never walked by the
/// inference engine (their intrinsic type registrations are deliberately
/// removed — `environment/mod.rs:1269`), so the span table carries no entry
/// for e.g. `__intrinsic_max(series)`. This is the type knowledge that the
/// deleted `builtin_return_numeric_type` register-feeder encoded — now
/// returned as a proper `Type` so the ONE Type model (and `numeric_type_of`
/// derived from it) sees the right kind. Mirrors the deleted table's set
/// exactly: `floor`/`ceil`/`round` → `int`; the rest → `number`.
fn builtin_function_return_type(name: &str) -> Option<shape_runtime::type_system::Type> {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::Type;
    let ty_name = match name {
        // Int-returning builtins (book spec: `(number) -> int`).
        "floor" | "ceil" | "round" => "int",
        // Number-returning builtins + `__intrinsic_*` stdlib-wrapper aliases.
        "abs"
        | "sqrt"
        | "sum"
        | "mean"
        | "min"
        | "max"
        | "sin"
        | "cos"
        | "tan"
        | "exp"
        | "ln"
        | "log"
        | "stddev"
        | "std"
        | "variance"
        | "pow"
        | "asin"
        | "acos"
        | "atan"
        | "__intrinsic_mean"
        | "__intrinsic_min"
        | "__intrinsic_max"
        | "__intrinsic_std"
        | "__intrinsic_variance"
        | "__intrinsic_correlation"
        | "__intrinsic_covariance"
        | "__intrinsic_percentile"
        | "__intrinsic_median" => "number",
        _ => return None,
    };
    Some(Type::Concrete(TypeAnnotation::Basic(ty_name.to_string())))
}

/// Extract the span from an expression (for source location tracking)
fn get_expr_span(expr: &Expr) -> Option<Span> {
    match expr {
        Expr::Literal(_, span)
        | Expr::Identifier(_, span)
        | Expr::Array(_, span)
        | Expr::Object(_, span)
        | Expr::Block(_, span)
        | Expr::Unit(span)
        | Expr::If(_, span)
        | Expr::While(_, span)
        | Expr::For(_, span)
        | Expr::Loop(_, span)
        | Expr::Match(_, span)
        | Expr::Let(_, span)
        | Expr::Assign(_, span)
        | Expr::TimeRef(_, span)
        | Expr::DateTime(_, span)
        | Expr::DataRef(_, span)
        | Expr::DataDateTimeRef(_, span)
        | Expr::Duration(_, span)
        | Expr::Spread(_, span)
        | Expr::ListComprehension(_, span)
        | Expr::TryOperator(_, span)
        | Expr::PatternRef(_, span)
        | Expr::WindowExpr(_, span)
        | Expr::FromQuery(_, span)
        | Expr::StructLiteral { span, .. } => Some(*span),

        Expr::BinaryOp { span, .. }
        | Expr::UnaryOp { span, .. }
        | Expr::FunctionCall { span, .. }
        | Expr::QualifiedFunctionCall { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::PropertyAccess { span, .. }
        | Expr::IndexAccess { span, .. }
        | Expr::Conditional { span, .. }
        | Expr::FuzzyComparison { span, .. }
        | Expr::EnumConstructor { span, .. }
        | Expr::TypeAssertion { span, .. }
        | Expr::TypeSyntax(_, span)
        | Expr::InstanceOf { span, .. }
        | Expr::Range { span, .. }
        | Expr::DataRelativeAccess { span, .. }
        | Expr::TimeframeContext { span, .. }
        | Expr::SimulationCall { span, .. }
        | Expr::FunctionExpr { span, .. } => Some(*span),

        Expr::Break(_, span)
        | Expr::Continue(span)
        | Expr::Return(_, span)
        | Expr::Await(_, span)
        | Expr::Join(_, span)
        | Expr::Annotated { span, .. }
        | Expr::UsingImpl { span, .. }
        | Expr::AsyncLet(_, span)
        | Expr::AsyncScope(_, span)
        | Expr::Comptime(_, span)
        | Expr::ComptimeFor(_, span)
        | Expr::Reference { span, .. }
        | Expr::TableRows(_, span) => Some(*span),
    }
}

#[cfg(test)]
mod u4_6_array_callable_tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::{BlockItem, Expr, Item, Program, Statement, TypeAnnotation};
    use shape_ast::parser::parse_program;
    use shape_runtime::type_system::Type;
    use shape_runtime::type_system::inference::TypeInferenceEngine;

    fn find_expr<'a>(program: &'a Program, pred: &impl Fn(&Expr) -> bool) -> Option<&'a Expr> {
        fn walk_expr<'a>(expr: &'a Expr, pred: &impl Fn(&Expr) -> bool) -> Option<&'a Expr> {
            if pred(expr) {
                return Some(expr);
            }
            match expr {
                Expr::BinaryOp { left, right, .. } => {
                    walk_expr(left, pred).or_else(|| walk_expr(right, pred))
                }
                Expr::MethodCall { receiver, args, .. } => walk_expr(receiver, pred)
                    .or_else(|| args.iter().find_map(|arg| walk_expr(arg, pred))),
                Expr::FunctionCall { args, .. } => args.iter().find_map(|arg| walk_expr(arg, pred)),
                Expr::IndexAccess { object, index, .. } => {
                    walk_expr(object, pred).or_else(|| walk_expr(index, pred))
                }
                Expr::Array(elements, _) => elements.iter().find_map(|elem| walk_expr(elem, pred)),
                Expr::Block(block, _) => block.items.iter().find_map(|item| match item {
                    BlockItem::VariableDecl(decl) => {
                        decl.value.as_ref().and_then(|value| walk_expr(value, pred))
                    }
                    BlockItem::Statement(stmt) => walk_stmt(stmt, pred),
                    BlockItem::Expression(expr) => walk_expr(expr, pred),
                    BlockItem::Assignment(_) => None,
                }),
                Expr::FunctionExpr { body, .. } => {
                    body.iter().find_map(|stmt| walk_stmt(stmt, pred))
                }
                _ => None,
            }
        }

        fn walk_stmt<'a>(stmt: &'a Statement, pred: &impl Fn(&Expr) -> bool) -> Option<&'a Expr> {
            match stmt {
                Statement::VariableDecl(decl, _) => {
                    decl.value.as_ref().and_then(|value| walk_expr(value, pred))
                }
                Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                    walk_expr(expr, pred)
                }
                _ => None,
            }
        }

        program.items.iter().find_map(|item| match item {
            Item::VariableDecl(decl, _) => {
                decl.value.as_ref().and_then(|value| walk_expr(value, pred))
            }
            Item::Statement(stmt, _) => walk_stmt(stmt, pred),
            Item::Function(func, _) => func.body.iter().find_map(|stmt| walk_stmt(stmt, pred)),
            Item::Expression(expr, _) => walk_expr(expr, pred),
            _ => None,
        })
    }

    fn is_int(ty: &Type) -> bool {
        matches!(ty, Type::Concrete(TypeAnnotation::Basic(name)) if name == "int")
    }

    #[test]
    fn indexed_callable_array_return_type_derives_from_inference_facts() {
        let program = parse_program(
            r#"
fn inc(x: int) -> int { x + 1 }
fn dbl(y: int) -> int { y + 2 }
let arr = [inc, dbl]
let total = arr[0](1) + arr[1](2)
"#,
        )
        .expect("program should parse");
        let total_expr = find_expr(&program, &|expr| {
            matches!(
                expr,
                Expr::BinaryOp { left, right, .. }
                    if matches!(left.as_ref(), Expr::MethodCall { method, .. } if method == "__call__")
                        && matches!(right.as_ref(), Expr::MethodCall { method, .. } if method == "__call__")
            )
        })
        .expect("total initializer");

        let mut engine = TypeInferenceEngine::new();
        let (facts, errors) = engine.infer_program_facts_best_effort(&program);
        assert!(
            errors.is_empty(),
            "indexed callable array program should infer cleanly, got {:?}",
            errors
        );
        let arr_span = facts
            .bindings_named("arr")
            .next()
            .expect("arr binding fact")
            .binder_span;

        let mut compiler = BytecodeCompiler::new();
        compiler.inference_facts = facts;
        compiler.module_bindings.insert("arr".to_string(), 0);
        compiler.module_binding_spans.insert(0, arr_span);

        let Expr::BinaryOp { left, .. } = total_expr else {
            panic!("total should be a binary op");
        };
        let helper_return = compiler
            .indexed_callable_array_return_type("arr", Some(1))
            .expect("indexed callable array helper should derive return type");
        assert!(
            is_int(&helper_return),
            "expected helper to derive int, got {:?}",
            helper_return
        );

        let call_return = compiler
            .infer_expr_type(left)
            .expect("left indexed callable call should type");
        assert!(
            is_int(&call_return),
            "expected arr[0](1) to infer int, got {:?}",
            call_return
        );
    }

    #[test]
    fn named_callable_binding_return_type_derives_from_inference_facts() {
        let program = parse_program(
            r#"
fn inc(x: int) -> int { x + 1 }
fn dbl(y: int) -> int { y + 2 }
let arr = [inc, dbl]
let g = arr[0]
let total = g(4) + 1
"#,
        )
        .expect("program should parse");
        let total_expr = find_expr(&program, &|expr| {
            matches!(
                expr,
                Expr::BinaryOp { left, .. }
                    if matches!(left.as_ref(), Expr::FunctionCall { name, .. } if name == "g")
            )
        })
        .expect("total initializer");

        let mut engine = TypeInferenceEngine::new();
        let (facts, errors) = engine.infer_program_facts_best_effort(&program);
        assert!(
            errors.is_empty(),
            "named callable binding program should infer cleanly, got {:?}",
            errors
        );
        let g_span = facts
            .bindings_named("g")
            .next()
            .expect("g binding fact")
            .binder_span;

        let mut compiler = BytecodeCompiler::new();
        compiler.inference_facts = facts;
        compiler.module_bindings.insert("g".to_string(), 0);
        compiler.module_binding_spans.insert(0, g_span);

        let helper_return = compiler
            .callable_binding_return_type("g", Some(1))
            .expect("callable binding helper should derive return type");
        assert!(
            is_int(&helper_return),
            "expected helper to derive int, got {:?}",
            helper_return
        );

        let Expr::BinaryOp { left, .. } = total_expr else {
            panic!("total should be a binary op");
        };
        let call_return = compiler
            .infer_expr_type(left)
            .expect("g(4) should type from binding facts");
        assert!(
            is_int(&call_return),
            "expected g(4) to infer int, got {:?}",
            call_return
        );
    }

    #[test]
    fn function_returning_closure_binding_compiles_without_return_string_map() {
        let program = parse_program(
            r#"
fn make(n: int) -> any {
    return fn(x: int) -> int { return x + 1 }
}
let f = make(7)
"#,
        )
        .expect("program should parse");

        let make_def = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(func, _) if func.name == "make" => Some(func.clone()),
                _ => None,
            })
            .expect("make function definition");
        let make_call = find_expr(
            &program,
            &|expr| matches!(expr, Expr::FunctionCall { name, .. } if name == "make"),
        )
        .expect("make call expression");

        let mut compiler = BytecodeCompiler::new();
        compiler.function_defs.insert("make".to_string(), make_def);
        compiler.module_bindings.insert("f".to_string(), 0);
        compiler.update_callable_binding_from_expr(0, false, make_call);

        let return_ty = compiler
            .callable_binding_return_type("f", Some(1))
            .expect("returned closure binding should derive return type from retained facts");
        assert!(
            is_int(&return_ty),
            "expected returned closure binding to derive int, got {:?}",
            return_ty
        );
    }
}

// Sub-modules organized by expression category
mod advanced;
mod assignment;
mod binary_ops;
pub(crate) mod closures;
mod collections;
mod conditionals;
mod control_flow;
mod data_access;
pub(crate) mod function_calls;
mod identifiers;
mod literals;
mod matrix_ops;
mod misc;
mod number_extend_specialization;
mod numeric_ops;
mod patterns;
mod property_access;
mod temporal;
mod type_ops;
mod unary_ops;

impl BytecodeCompiler {
    pub(crate) fn indexed_callable_array_return_type(
        &self,
        arr_name: &str,
        arg_count: Option<usize>,
    ) -> Option<shape_runtime::type_system::Type> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        let local_span = self
            .resolve_local(arr_name)
            .and_then(|local_idx| self.local_binding_spans.get(&local_idx).copied());
        let module_span = self
            .resolve_scoped_module_binding_name(arr_name)
            .and_then(|scoped| {
                self.module_bindings
                    .get(&scoped)
                    .and_then(|binding_idx| self.module_binding_spans.get(binding_idx).copied())
            })
            .or_else(|| {
                self.module_bindings
                    .get(arr_name)
                    .and_then(|binding_idx| self.module_binding_spans.get(binding_idx).copied())
            });
        let binding_type = local_span
            .or(module_span)
            .and_then(|span| self.inference_facts.binding_type(span))?;

        let canonical = binding_type.canonicalize();
        let element_ty = match canonical {
            Type::Generic { base, args } if args.len() == 1 => {
                let is_array = matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name))
                        if name.as_str() == "Array" || name.as_str() == "Vec"
                );
                if !is_array {
                    return None;
                }
                args.into_iter().next()?
            }
            _ => return None,
        };
        Self::callable_return_type_from_type(&element_ty, arg_count)
    }

    fn callable_return_type_from_type(
        callable_ty: &shape_runtime::type_system::Type,
        arg_count: Option<usize>,
    ) -> Option<shape_runtime::type_system::Type> {
        use shape_runtime::type_system::Type;
        let canonical = callable_ty.canonicalize();
        let Type::Function { params, returns } = canonical else {
            return None;
        };
        if let Some(expected) = arg_count
            && params.len() != expected
        {
            return None;
        }
        Some(*returns)
    }

    pub(crate) fn callable_binding_return_type(
        &mut self,
        name: &str,
        arg_count: Option<usize>,
    ) -> Option<shape_runtime::type_system::Type> {
        if let Some(local_idx) = self.resolve_local(name)
            && let Some(ty) = self.callable_local_slot_return_type(local_idx, arg_count)
        {
            return Some(ty);
        }
        if let Some(scoped) = self.resolve_scoped_module_binding_name(name)
            && let Some(binding_idx) = self.module_bindings.get(&scoped).copied()
            && let Some(ty) = self.callable_module_slot_return_type(binding_idx, arg_count)
        {
            return Some(ty);
        }
        if let Some(binding_idx) = self.module_bindings.get(name).copied()
            && let Some(ty) = self.callable_module_slot_return_type(binding_idx, arg_count)
        {
            return Some(ty);
        }
        None
    }

    fn callable_local_slot_return_type(
        &mut self,
        local_idx: u16,
        arg_count: Option<usize>,
    ) -> Option<shape_runtime::type_system::Type> {
        if let Some(span) = self.local_binding_spans.get(&local_idx).copied()
            && let Some(binding_ty) = self.inference_facts.binding_type(span)
            && let Some(return_ty) = Self::callable_return_type_from_type(binding_ty, arg_count)
            && !Self::type_contains_unknown(&return_ty)
        {
            return Some(return_ty);
        }

        let peek = self
            .local_callable_closure_bodies
            .get(&local_idx)
            .cloned()?;
        let return_ty =
            crate::compiler::expressions::closures::infer_closure_body_return_type_with_caller_context(
                self,
                &peek.params,
                &peek.body,
                peek.return_type.as_ref(),
                &[],
                &[],
            )?;
        if Self::type_contains_unknown(&return_ty) {
            return None;
        }
        Some(return_ty)
    }

    fn callable_module_slot_return_type(
        &mut self,
        binding_idx: u16,
        arg_count: Option<usize>,
    ) -> Option<shape_runtime::type_system::Type> {
        if let Some(span) = self.module_binding_spans.get(&binding_idx).copied()
            && let Some(binding_ty) = self.inference_facts.binding_type(span)
            && let Some(return_ty) = Self::callable_return_type_from_type(binding_ty, arg_count)
            && !Self::type_contains_unknown(&return_ty)
        {
            return Some(return_ty);
        }

        let peek = self
            .module_binding_callable_closure_bodies
            .get(&binding_idx)
            .cloned()?;
        let return_ty =
            crate::compiler::expressions::closures::infer_closure_body_return_type_with_caller_context(
                self,
                &peek.params,
                &peek.body,
                peek.return_type.as_ref(),
                &[],
                &[],
            )?;
        if Self::type_contains_unknown(&return_ty) {
            return None;
        }
        Some(return_ty)
    }

    fn type_contains_unknown(ty: &shape_runtime::type_system::Type) -> bool {
        use shape_runtime::type_system::Type;
        match ty {
            Type::Concrete(ann) => Self::annotation_contains_unknown(ann),
            Type::Generic { base, args } => {
                Self::type_contains_unknown(base) || args.iter().any(Self::type_contains_unknown)
            }
            Type::Function { params, returns } => {
                params.iter().any(Self::type_contains_unknown)
                    || Self::type_contains_unknown(returns)
            }
            Type::Variable(_) | Type::Constrained { .. } => true,
        }
    }

    fn annotation_contains_unknown(ann: &shape_ast::ast::TypeAnnotation) -> bool {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Basic(name) => name == "unknown",
            TypeAnnotation::Reference(path) => path.as_str() == "unknown",
            TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
                Self::annotation_contains_unknown(inner)
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => {
                items.iter().any(Self::annotation_contains_unknown)
            }
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| Self::annotation_contains_unknown(&field.type_annotation)),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| Self::annotation_contains_unknown(&param.type_annotation))
                    || Self::annotation_contains_unknown(returns)
            }
            TypeAnnotation::Generic { name, args } => {
                name.as_str() == "unknown" || args.iter().any(Self::annotation_contains_unknown)
            }
            TypeAnnotation::Dyn(paths) => paths.iter().any(|path| path.as_str() == "unknown"),
            // ADR-009 B3 (S1): existential descriptor package — check the inner
            // descriptor for `unknown` holes.
            TypeAnnotation::Existential { inner, .. } => Self::annotation_contains_unknown(inner),
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => false,
        }
    }

    fn annotation_target_kind_for_expr(
        target: &Expr,
        forced_kind: Option<shape_ast::ast::functions::AnnotationTargetKind>,
    ) -> shape_ast::ast::functions::AnnotationTargetKind {
        if let Some(kind) = forced_kind {
            return kind;
        }
        match target {
            Expr::Annotated {
                target: inner_target,
                ..
            } => Self::annotation_target_kind_for_expr(inner_target, None),
            Expr::Block(..) => shape_ast::ast::functions::AnnotationTargetKind::Block,
            Expr::Let(..) => shape_ast::ast::functions::AnnotationTargetKind::Binding,
            _ => shape_ast::ast::functions::AnnotationTargetKind::Expression,
        }
    }

    fn comptime_target_kind_for_annotation(
        kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> super::comptime_target::AnnotationTargetKind {
        match kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => {
                super::comptime_target::AnnotationTargetKind::Function
            }
            shape_ast::ast::functions::AnnotationTargetKind::Type => {
                super::comptime_target::AnnotationTargetKind::Type
            }
            shape_ast::ast::functions::AnnotationTargetKind::Module => {
                super::comptime_target::AnnotationTargetKind::Module
            }
            shape_ast::ast::functions::AnnotationTargetKind::Expression => {
                super::comptime_target::AnnotationTargetKind::Expression
            }
            shape_ast::ast::functions::AnnotationTargetKind::Block => {
                super::comptime_target::AnnotationTargetKind::Block
            }
            shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr => {
                super::comptime_target::AnnotationTargetKind::AwaitExpr
            }
            shape_ast::ast::functions::AnnotationTargetKind::Binding => {
                super::comptime_target::AnnotationTargetKind::Binding
            }
        }
    }

    fn annotation_target_name(target: &Expr) -> String {
        match target {
            Expr::Identifier(name, _) => name.clone(),
            Expr::Let(let_expr, _) => let_expr
                .pattern
                .as_simple_name()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn run_comptime_annotation_handlers_for_target(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> Result<bool> {
        if let Some((_, compiled)) = self.lookup_compiled_annotation(annotation) {
            let handlers = [
                compiled.comptime_pre_handler,
                compiled.comptime_post_handler,
            ];
            for handler in handlers.into_iter().flatten() {
                let mut target_desc = super::comptime_target::ComptimeTarget::for_expression();
                target_desc.kind = Self::comptime_target_kind_for_annotation(target_kind);
                target_desc.name = Self::annotation_target_name(target);
                target_desc.annotations = vec![annotation.name.clone()];
                let target_name = if target_desc.name.is_empty() {
                    "target".to_string()
                } else {
                    target_desc.name.clone()
                };
                // ADR-009 D1 (S2): expansion site for this expression-target
                // handler application.
                let expansion_site =
                    self.annotation_expansion_site(annotation, &handler, &target_desc);
                // R8 W9 G.2 Step 2 Bucket 7: to_nanboxed now returns
                // Result; surface the V3-S5 ckpt-5 SURFACE through the
                // caller's Result chain instead of panicking. E1 slice-5: an
                // expression target has no members/AST → `None` overlay, every
                // stamp INVALID.
                let target_value = target_desc.to_nanboxed(None)?;
                let handler_span = handler.span;
                // ADR-009 E1 #17 (slice 5): the handler executor still needs a
                // freeze handle; acquire it here (no target stamping occurred).
                let freeze = self.comptime_freeze_overlay()?;
                // ADR-009 C3 #14 (slice 4): the def-param carrier reads the
                // FULL param definitions (declared types ride along).
                let def_params =
                    crate::compiler::functions_annotations::handler_resolution::annotation_def_params(
                        &compiled.param_defs,
                    );
                let execution = self.execute_comptime_annotation_handler(
                    annotation,
                    &handler,
                    target_value,
                    &def_params,
                    &[],
                    // Expression target: no representation authority (Dec 56).
                    None,
                    freeze,
                )?;

                let removed = self
                    .process_comptime_directives(
                        execution.directives,
                        &target_name,
                        &expansion_site,
                    )
                    .map_err(|e| {
                        // ADR-009 D1 (S4): provenance-carrying generated-decl
                        // failures pass through with their location notes.
                        self.preserve_or_wrap_directive_failure(
                            e,
                            &format!("Comptime handler '{}'", annotation.name),
                            handler_span,
                        )
                    })?;

                if removed {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    self.last_expr_schema = None;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Apply the before-handler result contract.
    ///
    /// The before handler can return:
    /// - An array → replaces args
    /// - An object `{ args?, state?, result? }` → updates args/state, and if
    ///   `result` is non-null, short-circuits (skips impl call / expression eval)
    ///
    /// When `result_local` is `Some`, the `result` field is extracted and stored
    /// there, and a short-circuit jump is emitted. The returned `Option<usize>`
    /// is the jump address that must be patched by the caller to skip past the
    /// impl call / expression evaluation.
    fn apply_before_result_contract(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
    ) -> Result<()> {
        self.apply_before_result_contract_inner(
            before_result_local,
            args_local,
            ctx_local,
            ctx_schema_id,
            None,
        )
        .map(|_| ())
    }

    /// Like `apply_before_result_contract` but with short-circuit support.
    ///
    /// When `short_circuit_result_local` is provided, the `result` field of the
    /// before-handler object is extracted. If non-null, the value is stored in
    /// the given local and a jump is emitted. The returned `Option<usize>` is
    /// the jump that must be patched to skip past the impl/expression.
    fn apply_before_result_contract_with_short_circuit(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
        short_circuit_result_local: u16,
    ) -> Result<Option<usize>> {
        self.apply_before_result_contract_inner(
            before_result_local,
            args_local,
            ctx_local,
            ctx_schema_id,
            Some(short_circuit_result_local),
        )
    }

    fn apply_before_result_contract_inner(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
        short_circuit_result_local: Option<u16>,
    ) -> Result<Option<usize>> {
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        let one_const = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsArray)),
        ));
        let skip_array = self.emit_jump(OpCode::JumpIfFalse, 0);
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(args_local)),
        ));
        let skip_obj_check = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(skip_array);

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        let one_const2 = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const2)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsObject)),
        ));
        let skip_obj = self.emit_jump(OpCode::JumpIfFalse, 0);

        // Schema includes `result` field for short-circuit support
        let before_contract_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("args", FieldType::Any),
            ("result", FieldType::Any),
            ("state", FieldType::Any),
        ]);
        let (args_operand, state_operand, result_operand) = {
            let schema = self
                .type_tracker
                .schema_registry()
                .get_by_id(before_contract_schema_id)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: missing before-handler schema".to_string(),
                    location: None,
                })?;
            let args_field = schema
                .get_field("args")
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: before-handler schema missing 'args'".to_string(),
                    location: None,
                })?;
            let state_field =
                schema
                    .get_field("state")
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: "Internal error: before-handler schema missing 'state'"
                            .to_string(),
                        location: None,
                    })?;
            let result_field =
                schema
                    .get_field("result")
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: "Internal error: before-handler schema missing 'result'"
                            .to_string(),
                        location: None,
                    })?;
            if args_field.offset > u16::MAX as usize
                || state_field.offset > u16::MAX as usize
                || result_field.offset > u16::MAX as usize
            {
                return Err(ShapeError::RuntimeError {
                    message: "Internal error: before-handler field offset/index overflow"
                        .to_string(),
                    location: None,
                });
            }
            (
                Operand::TypedField {
                    type_id: before_contract_schema_id as u16,
                    field_idx: args_field.index as u16,
                    field_type_tag: field_type_to_tag(&args_field.field_type),
                },
                Operand::TypedField {
                    type_id: before_contract_schema_id as u16,
                    field_idx: state_field.index as u16,
                    field_type_tag: field_type_to_tag(&state_field.field_type),
                },
                Operand::TypedField {
                    type_id: before_contract_schema_id as u16,
                    field_idx: result_field.index as u16,
                    field_type_tag: field_type_to_tag(&result_field.field_type),
                },
            )
        };

        // Check `result` field for short-circuit
        let mut short_circuit_jump = None;
        if let Some(sc_local) = short_circuit_result_local {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result_local)),
            ));
            self.emit(Instruction::new(
                OpCode::GetFieldTyped,
                Some(result_operand),
            ));
            // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let skip_short_circuit = self.emit_jump(OpCode::JumpIfTrue, 0);
            // result is non-null → store it and jump past impl
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(sc_local)),
            ));
            short_circuit_jump = Some(self.emit_jump(OpCode::Jump, 0));
            self.patch_jump(skip_short_circuit);
            self.emit(Instruction::simple(OpCode::Pop)); // discard null result
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(args_operand)));
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsNull));
        let skip_args_replace = self.emit_jump(OpCode::JumpIfTrue, 0);
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(args_local)),
        ));
        let skip_pop_args = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(skip_args_replace);
        self.emit(Instruction::simple(OpCode::Pop));
        self.patch_jump(skip_pop_args);

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(state_operand)));
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsNull));
        let skip_state = self.emit_jump(OpCode::JumpIfTrue, 0);
        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: ctx_schema_id as u16,
                field_count: 2,
            }),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(ctx_local)),
        ));
        let skip_pop_state = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(skip_state);
        self.emit(Instruction::simple(OpCode::Pop));
        self.patch_jump(skip_pop_state);

        self.patch_jump(skip_obj);
        self.patch_jump(skip_obj_check);
        Ok(short_circuit_jump)
    }

    fn compile_annotated_expr(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        ann_span: Span,
        forced_kind: Option<shape_ast::ast::functions::AnnotationTargetKind>,
    ) -> Result<()> {
        let target_kind = Self::annotation_target_kind_for_expr(target, forced_kind);
        self.validate_annotation_target_usage(annotation, target_kind, ann_span)?;
        if self.run_comptime_annotation_handlers_for_target(annotation, target, target_kind)? {
            return Ok(());
        }

        if let Some(compiled) = self
            .program
            .compiled_annotations
            .get(&annotation.name)
            .cloned()
        {
            if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                self.push_scope();
                let args_local = self.declare_hygienic_local(HygienicRole::AnnotationArgs)?;
                let ctx_local = self.declare_hygienic_local(HygienicRole::AnnotationCtx)?;
                let result_local = self.declare_hygienic_local(HygienicRole::AnnotationResult)?;

                // Build args array for expression annotations.
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(args_local)),
                ));

                // Build ctx object: { state: {}, event_log: [] }.
                // W17.2-C §4.D.5 migration: empty-fields case uses typed variant.
                let empty_schema_id = self.type_tracker.register_inline_object_schema_typed(&[]);
                self.emit(Instruction::new(
                    OpCode::NewTypedObject,
                    Some(Operand::TypedObjectAlloc {
                        schema_id: empty_schema_id as u16,
                        field_count: 0,
                    }),
                ));
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
                    ("state", FieldType::Any),
                    ("event_log", FieldType::Array(Box::new(FieldType::Any))),
                ]);
                self.emit(Instruction::new(
                    OpCode::NewTypedObject,
                    Some(Operand::TypedObjectAlloc {
                        schema_id: ctx_schema_id as u16,
                        field_count: 2,
                    }),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(ctx_local)),
                ));

                if let Some(before_id) = compiled.before_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
                        self.compile_expr(ann_arg)?;
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(args_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(ctx_local)),
                    ));
                    let before_arg_count = 1 + annotation.args.len() + 2;
                    let count_const = self
                        .program
                        .add_constant(Constant::Int(before_arg_count as i64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(before_id))),
                    ));
                    self.record_blob_call(before_id);

                    let before_result_local =
                        self.declare_hygienic_local(HygienicRole::AnnotationBeforeResult)?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(before_result_local)),
                    ));
                    self.apply_before_result_contract(
                        before_result_local,
                        args_local,
                        ctx_local,
                        ctx_schema_id,
                    )?;
                }

                if let Expr::Annotated {
                    annotation: inner_annotation,
                    target: inner_target,
                    span: inner_span,
                } = target
                {
                    self.compile_annotated_expr(
                        inner_annotation,
                        inner_target,
                        *inner_span,
                        forced_kind,
                    )?;
                } else {
                    self.compile_expr(target)?;
                }
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                if let Some(after_id) = compiled.after_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
                        self.compile_expr(ann_arg)?;
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(args_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(result_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(ctx_local)),
                    ));
                    let after_arg_count = 1 + annotation.args.len() + 3;
                    let count_const = self
                        .program
                        .add_constant(Constant::Int(after_arg_count as i64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(after_id))),
                    ));
                    self.record_blob_call(after_id);
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(result_local)),
                    ));
                }

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
                self.stamp_awaited_future_payload_type(target);
                self.pop_scope();
                return Ok(());
            }
        }

        if let Expr::Annotated {
            annotation: inner_annotation,
            target: inner_target,
            span: inner_span,
        } = target
        {
            self.compile_annotated_expr(inner_annotation, inner_target, *inner_span, forced_kind)
        } else {
            self.compile_expr(target)
        }
    }

    fn compile_annotated_await_expr(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        ann_span: Span,
    ) -> Result<()> {
        let target_kind = shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr;
        self.validate_annotation_target_usage(annotation, target_kind, ann_span)?;

        if self.run_comptime_annotation_handlers_for_target(annotation, target, target_kind)? {
            return Ok(());
        }

        if let Some(compiled) = self
            .program
            .compiled_annotations
            .get(&annotation.name)
            .cloned()
        {
            if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                self.push_scope();
                let args_local = self.declare_hygienic_local(HygienicRole::AnnotationArgs)?;
                let ctx_local = self.declare_hygienic_local(HygienicRole::AnnotationCtx)?;
                let subject_local = self.declare_hygienic_local(HygienicRole::AnnotationSubject)?;
                let result_local = self.declare_hygienic_local(HygienicRole::AnnotationResult)?;

                // W17.2-C §4.D.5 migration: empty-fields case uses typed variant.
                let empty_schema_id = self.type_tracker.register_inline_object_schema_typed(&[]);
                self.emit(Instruction::new(
                    OpCode::NewTypedObject,
                    Some(Operand::TypedObjectAlloc {
                        schema_id: empty_schema_id as u16,
                        field_count: 0,
                    }),
                ));
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
                    ("state", FieldType::Any),
                    ("event_log", FieldType::Array(Box::new(FieldType::Any))),
                ]);
                self.emit(Instruction::new(
                    OpCode::NewTypedObject,
                    Some(Operand::TypedObjectAlloc {
                        schema_id: ctx_schema_id as u16,
                        field_count: 2,
                    }),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(ctx_local)),
                ));

                // Initialize args as empty array (before handler gets annotation
                // args + ctx, not the evaluated expression)
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(args_local)),
                ));

                // Call before handler FIRST (before evaluating inner expression).
                // This allows short-circuit: if before returns { result: value },
                // we skip the inner expression eval + await entirely.
                let mut short_circuit_jump = None;
                if let Some(before_id) = compiled.before_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
                        self.compile_expr(ann_arg)?;
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(args_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(ctx_local)),
                    ));
                    let before_arg_count = 1 + annotation.args.len() + 2;
                    let count_const = self
                        .program
                        .add_constant(Constant::Int(before_arg_count as i64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(before_id))),
                    ));
                    self.record_blob_call(before_id);

                    let before_result_local =
                        self.declare_hygienic_local(HygienicRole::AnnotationBeforeResult)?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(before_result_local)),
                    ));
                    short_circuit_jump = self.apply_before_result_contract_with_short_circuit(
                        before_result_local,
                        args_local,
                        ctx_local,
                        ctx_schema_id,
                        result_local,
                    )?;
                }

                // --- Normal path: evaluate inner expression + await ---
                if let Expr::Annotated {
                    annotation: inner_annotation,
                    target: inner_target,
                    span: inner_span,
                } = target
                {
                    self.compile_annotated_await_expr(inner_annotation, inner_target, *inner_span)?;
                } else {
                    self.compile_expr(target)?;
                }
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(subject_local)),
                ));

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(subject_local)),
                ));
                self.emit(Instruction::simple(OpCode::Await));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                // Patch the short-circuit jump to land here (after await, at result usage)
                if let Some(jump_addr) = short_circuit_jump {
                    self.patch_jump(jump_addr);
                }

                if let Some(after_id) = compiled.after_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
                        self.compile_expr(ann_arg)?;
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(args_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(result_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(ctx_local)),
                    ));
                    let after_arg_count = 1 + annotation.args.len() + 3;
                    let count_const = self
                        .program
                        .add_constant(Constant::Int(after_arg_count as i64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(after_id))),
                    ));
                    self.record_blob_call(after_id);
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(result_local)),
                    ));
                }

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
                self.stamp_awaited_future_payload_type(target);
                self.pop_scope();
                return Ok(());
            }
        }

        if let Expr::Annotated {
            annotation: inner_annotation,
            target: inner_target,
            span: inner_span,
        } = target
        {
            self.compile_annotated_await_expr(inner_annotation, inner_target, *inner_span)?;
        } else {
            self.compile_expr(target)?;
        }
        self.emit(Instruction::simple(OpCode::Await));
        self.stamp_awaited_future_payload_type(target);
        Ok(())
    }

    pub(super) fn capture_last_expr_reference_result(&self) -> ExprReferenceResult {
        self.last_expr_reference_result
    }

    pub(super) fn restore_last_expr_reference_result(&mut self, result: ExprReferenceResult) {
        self.last_expr_reference_result = result;
    }

    pub(super) fn clear_last_expr_reference_result(&mut self) {
        self.last_expr_reference_result = ExprReferenceResult::default();
    }

    pub(super) fn set_last_expr_reference_result(&mut self, mode: BorrowMode, auto_deref: bool) {
        self.last_expr_reference_result = ExprReferenceResult {
            raw_mode: Some(mode),
            auto_deref_mode: auto_deref.then_some(mode),
        };
    }

    pub(super) fn last_expr_reference_mode(&self) -> Option<BorrowMode> {
        self.last_expr_reference_result.raw_mode
    }

    pub(super) fn merge_reference_results(results: &[ExprReferenceResult]) -> ExprReferenceResult {
        let Some(first) = results.first().copied() else {
            return ExprReferenceResult::default();
        };
        let Some(raw_mode) = first.raw_mode else {
            return ExprReferenceResult::default();
        };
        if !results
            .iter()
            .all(|result| result.raw_mode == Some(raw_mode))
        {
            return ExprReferenceResult::default();
        }
        let auto_deref_mode = if first.auto_deref_mode.is_some()
            && results
                .iter()
                .all(|result| result.auto_deref_mode == first.auto_deref_mode)
        {
            first.auto_deref_mode
        } else {
            None
        };
        ExprReferenceResult {
            raw_mode: Some(raw_mode),
            auto_deref_mode,
        }
    }

    fn auto_deref_last_expr_result_if_needed(&mut self) -> Result<()> {
        if self.last_expr_reference_result.auto_deref_mode.is_none() {
            return Ok(());
        }
        let temp = self.declare_temp_local("__expr_auto_deref_")?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp)),
        ));
        self.emit(Instruction::new(
            OpCode::DerefLoad,
            Some(Operand::Local(temp)),
        ));
        self.clear_last_expr_reference_result();
        Ok(())
    }

    pub(super) fn current_expr_result_mode(&self) -> ExprResultMode {
        self.current_expr_result_mode
    }

    /// Named arguments (STAGE T4, 2026-06-22): bind a free-function call's
    /// `name: value` arguments to the callee's parameters BY NAME, producing a
    /// fully positional `Vec<Expr>` that the existing call-lowering path
    /// (`compile_expr_function_call`) compiles unchanged.
    ///
    /// Returns `Ok(None)` when there are no named args (the caller keeps its
    /// original positional `args` slice — zero behavioural change). Returns
    /// `Ok(Some(positional))` when named args were present and successfully
    /// rebound: `positional[i]` is the supplied (positional or named) argument
    /// for parameter `i`, or that parameter's declared `default_value` when it
    /// was omitted. The vec is dense — trailing omitted params WITHOUT a
    /// default are left off, so the downstream arity check still produces its
    /// "expects between X and Y" diagnostic.
    ///
    /// Clean compile-errors (ADR-006 surface-and-stop, never silent
    /// miscompute):
    ///  - named args on a non-user function (builtin / enum ctor / local
    ///    callable): named binding is unsupported there;
    ///  - an unknown named arg (no matching parameter name);
    ///  - a parameter bound twice (positional+named, or duplicate named).
    ///
    /// `name` is the surface call name; param names + defaults come from
    /// `self.function_defs` (monomorphization/const-specialization never
    /// renames parameters, so the surface name is the correct key).
    pub(super) fn resolve_named_function_args(
        &self,
        name: &str,
        args: &[Expr],
        named_args: &[(String, Expr)],
        span: shape_ast::ast::Span,
    ) -> Result<Option<Vec<Expr>>> {
        if named_args.is_empty() {
            return Ok(None);
        }

        // Named binding is only defined for user functions whose parameter
        // names are statically known. Anything else (builtins, enum
        // constructors, local callable values) cannot resolve names → reject.
        let Some(def) = self.function_defs.get(name) else {
            let names = named_args
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Named call arguments are not supported on `{name}` \
                     (named argument(s): {names}). Pass arguments positionally."
                ),
                location: Some(self.span_to_source_location(span)),
            });
        };

        let params = def.params.clone();
        let param_names: Vec<Option<String>> = params
            .iter()
            .map(|p| p.simple_name().map(|s| s.to_string()))
            .collect();
        let n_params = params.len();

        // Slot per parameter; filled from positional first, then named.
        let mut slots: Vec<Option<Expr>> = vec![None; n_params];

        if args.len() > n_params {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Function '{}' expects at most {} positional argument(s), got {}",
                    name,
                    n_params,
                    args.len()
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        for (i, arg) in args.iter().enumerate() {
            slots[i] = Some(arg.clone());
        }

        for (arg_name, arg_expr) in named_args {
            let Some(idx) = param_names
                .iter()
                .position(|p| p.as_deref() == Some(arg_name.as_str()))
            else {
                return Err(ShapeError::SemanticError {
                    message: format!("Function '{name}' has no parameter named '{arg_name}'"),
                    location: Some(self.span_to_source_location(span)),
                });
            };
            if slots[idx].is_some() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Argument for parameter '{arg_name}' of function '{name}' \
                         was supplied more than once (positional and/or named)"
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            slots[idx] = Some(arg_expr.clone());
        }

        // Fill omitted parameters that carry a `default_value`. Leave a
        // trailing run of unfilled-without-default slots OFF the end so the
        // downstream arity check reports the missing required arguments; an
        // INTERIOR hole with no default (a later slot is filled) is a clean
        // error here — the positional path could not express it.
        for (idx, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                if let Some(default_expr) = params[idx].default_value.clone() {
                    *slot = Some(default_expr);
                }
            }
        }

        // Highest filled index — everything up to it must be present.
        let last_filled = slots.iter().rposition(|s| s.is_some());
        let mut positional: Vec<Expr> = Vec::with_capacity(n_params);
        if let Some(last) = last_filled {
            for (idx, slot) in slots.into_iter().enumerate().take(last + 1) {
                match slot {
                    Some(expr) => positional.push(expr),
                    None => {
                        let pname = param_names
                            .get(idx)
                            .and_then(|p| p.clone())
                            .unwrap_or_else(|| format!("#{}", idx + 1));
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Function '{name}' is missing a value for parameter \
                                 '{pname}' (it has no default and was not supplied \
                                 positionally or by name)"
                            ),
                            location: Some(self.span_to_source_location(span)),
                        });
                    }
                }
            }
        }

        Ok(Some(positional))
    }

    pub(super) fn compile_expr_preserving_refs(&mut self, expr: &Expr) -> Result<()> {
        let saved_mode = self.current_expr_result_mode;
        self.current_expr_result_mode = ExprResultMode::PreserveRef;
        self.clear_last_expr_reference_result();

        let result = match expr {
            Expr::Identifier(name, span) => {
                self.compile_expr_identifier_preserving_refs(name, *span)
            }
            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                span,
                ..
            } => {
                let rebound = self.resolve_named_function_args(name, args, named_args, *span)?;
                let args: &[Expr] = rebound.as_deref().unwrap_or(args);
                self.compile_expr_function_call(name, const_args, args, *span)
            }
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                const_args,
                args,
                span,
                ..
            } => self
                .compile_expr_qualified_function_call(namespace, function, const_args, args, *span),
            Expr::MethodCall {
                receiver,
                method,
                args,
                span,
                ..
            } => self.compile_expr_method_call(receiver, method, args, *span),
            Expr::Reference {
                expr: inner,
                is_mutable,
                span,
            } => {
                let mode = if *is_mutable {
                    BorrowMode::Exclusive
                } else {
                    BorrowMode::Shared
                };
                let result = self.compile_reference_expr(inner, *span, mode).map(|_| ());
                if result.is_ok() {
                    self.set_last_expr_reference_result(mode, false);
                }
                result
            }
            Expr::Block(block, _) => self.compile_expr_block(block),
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.compile_expr_conditional(condition, then_expr, else_expr),
            Expr::If(if_expr, _) => self.compile_expr_if(if_expr),
            Expr::Let(let_expr, _) => self.compile_expr_let(let_expr),
            Expr::Assign(assign_expr, _) => self.compile_expr_assign(assign_expr),
            Expr::Match(match_expr, _) => self.compile_expr_match(match_expr),
            _ => {
                let result = self.compile_expr(expr);
                if result.is_ok() {
                    self.clear_last_expr_reference_result();
                }
                result
            }
        };

        self.current_expr_result_mode = saved_mode;
        result
    }

    /// Main expression compilation dispatcher
    ///
    /// This method dispatches to specialized compilation methods based on expression type.
    pub(super) fn compile_expr(&mut self, expr: &Expr) -> Result<()> {
        let saved_mode = self.current_expr_result_mode;
        self.current_expr_result_mode = ExprResultMode::Value;
        self.clear_last_expr_reference_result();

        // Reset numeric type tracking — each expression must explicitly set it.
        // Without this, a stale numeric type from a previous sub-expression
        // could cause the wrong typed opcode to be emitted.
        self.last_expr_schema = None;
        self.last_expr_type_info = None;

        // Track source line from expression span for error messages
        if let Some(span) = get_expr_span(expr) {
            self.set_line_from_span(span);
        }

        let result = match expr {
            // Literals
            Expr::Literal(lit, _) => self.compile_expr_literal(lit),

            // Identifiers
            Expr::Identifier(name, span) => self.compile_expr_identifier(name, *span),

            // Binary operations
            // W10 jit-call-method-user-trait-fix (2026-05-17): thread the
            // outer Expr::BinaryOp.span (parser pair span — matches the
            // MIR lowering's `lower_expr_to_temp` `expr.span()` for the
            // `Rvalue::BinaryOp` statement at `crates/shape-vm/src/mir/
            // lowering/expr.rs:1716`) so the operator-trait-dispatch
            // side-table keys align across the bytecode / MIR / JIT
            // surfaces.
            Expr::BinaryOp {
                left, op, right, span,
            } => self.compile_expr_binary_op(left, op, right, *span),

            // Fuzzy comparison (compile left and right, then apply fuzzy comparison)
            Expr::FuzzyComparison {
                left,
                op,
                right,
                tolerance,
                ..
            } => self.compile_expr_fuzzy_comparison(left, op, right, tolerance),

            // Unary operations — sibling of the BinaryOp span threading
            // above (W10 jit-call-method-user-trait-fix).
            Expr::UnaryOp { op, operand, span } => {
                self.compile_expr_unary_op(op, operand, *span)
            }

            // ADR-009 A2: the type-syntax carrier is consumed by the
            // comptime type_ref rewrite before codegen. Reaching codegen
            // means it sits outside the type_ref argument position (or the
            // rewrite did not run) — a named surface-and-stop error, never
            // a silently compiled value.
            Expr::TypeSyntax(_, _) => Err(ShapeError::SemanticError {
                message: "type syntax is only valid as the type_ref argument".to_string(),
                location: None,
            }),

            // Type operations
            Expr::TypeAssertion {
                expr,
                type_annotation,
                ..
            } => self.compile_expr_type_assertion(expr, type_annotation),
            Expr::InstanceOf {
                expr,
                type_annotation,
                ..
            } => self.compile_expr_instanceof(expr, type_annotation),

            // Collections
            Expr::Array(elements, span) => self.compile_expr_array(elements, *span),
            Expr::Object(fields, _) => self.compile_expr_object(fields),

            // Property and index access
            Expr::PropertyAccess {
                object,
                property,
                optional,
                ..
            } => self.compile_expr_property_access(object, property, *optional),
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => self.compile_expr_index_access(object, index, end_index),

            // Function calls
            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                span,
                ..
            } => {
                let rebound =
                    self.resolve_named_function_args(name, args, named_args, *span)?;
                let args: &[Expr] = rebound.as_deref().unwrap_or(args);
                self.compile_expr_function_call(name, const_args, args, *span)
            }
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                const_args,
                args,
                span,
                ..
            } => self.compile_expr_qualified_function_call(namespace, function, const_args, args, *span),
            Expr::MethodCall {
                receiver,
                method,
                args,
                span,
                ..
            } => self.compile_expr_method_call(receiver, method, args, *span),
            Expr::EnumConstructor {
                enum_name,
                variant,
                payload,
                span,
                ..
            } => {
                // Check if this is a Type::comptime_field access (looks like enum syntax)
                if matches!(payload, shape_ast::ast::EnumConstructorPayload::Unit) {
                    if let Some(slot) = self
                        .comptime_fields
                        .get(enum_name.as_str())
                        .and_then(|m| m.get(variant))
                        .cloned()
                    {
                        return self.emit_comptime_field_constant(
                            enum_name, variant, &slot, *span,
                        );
                    }
                }
                self.compile_expr_enum_constructor(enum_name, variant, payload)
            }

            // Closures. ADR-009 C1 (slice 2): the node's own generated-code
            // provenance is THE Wave-46 capture-gate predicate — pass it in
            // rather than re-deriving "am I in generated code?" from the
            // enclosing function's NAME.
            Expr::FunctionExpr {
                params,
                body,
                generated_origin,
                captures,
                annotations,
                span,
                ..
            } => {
                // ADR-009 C3 #14 (slice 4, C3-G12): annotations on a fn-local
                // NESTED `fn` (carried by the parser desugar) — a TypedConfig
                // (hook-template) annotation is a LOUD named rejection at the
                // application site; legacy-classified annotations keep the
                // pre-slice-4 silent drop until S5's matrix owns the class.
                if let Some(annotations) = annotations.as_deref() {
                    self.reject_typed_config_annotations_on_nested_fn(annotations)?;
                }
                self.compile_expr_closure(
                    params,
                    body,
                    captures.as_deref(),
                    generated_origin.as_deref(),
                    *span,
                )
            }

            // Conditionals
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.compile_expr_conditional(condition, then_expr, else_expr),
            Expr::If(if_expr, _) => self.compile_expr_if(if_expr),

            // Loops
            Expr::While(while_expr, _) => self.compile_expr_while(while_expr),
            Expr::For(for_expr, _) => self.compile_expr_for(for_expr),
            Expr::Loop(loop_expr, _) => self.compile_expr_loop(loop_expr),

            // Data access
            Expr::DataRef(data_ref, _) => self.compile_expr_data_ref(data_ref),
            Expr::DataDateTimeRef(datetime_ref, _) => {
                self.compile_expr_data_datetime_ref(datetime_ref)
            }
            Expr::DataRelativeAccess {
                reference, index, ..
            } => self.compile_expr_data_relative_access(reference, index),

            // Temporal
            Expr::TimeRef(time_ref, _) => self.compile_expr_time_ref(time_ref),
            Expr::DateTime(datetime_expr, _) => self.compile_expr_datetime(datetime_expr),
            Expr::Duration(duration, _) => self.compile_expr_duration(duration),
            Expr::TimeframeContext {
                timeframe, expr, ..
            } => self.compile_expr_timeframe_context(*timeframe, expr),

            // Control flow
            Expr::Break(value_expr, _) => self.compile_expr_break(value_expr),
            Expr::Continue(_) => self.compile_expr_continue(),
            Expr::Return(value_expr, _) => self.compile_expr_return(value_expr),

            // Let and assignment
            Expr::Let(let_expr, _) => self.compile_expr_let(let_expr),
            Expr::Assign(assign_expr, _) => self.compile_expr_assign(assign_expr),

            // Advanced expressions
            Expr::ListComprehension(comp, _) => self.compile_expr_list_comprehension(comp),
            Expr::TryOperator(inner, _) => self.compile_expr_try_operator(inner),
            Expr::UsingImpl {
                expr, impl_name, ..
            } => self.compile_expr_using_impl(expr, impl_name),
            Expr::Match(match_expr, _) => self.compile_expr_match(match_expr),

            // Pattern references
            Expr::PatternRef(name, _) => self.compile_expr_pattern_ref(name),

            // Miscellaneous
            Expr::Unit(_) => self.compile_expr_unit(),
            Expr::Spread(..) => self.compile_expr_spread(),
            Expr::Block(block, _) => self.compile_expr_block(block),
            Expr::Range {
                start, end, kind, ..
            } => self.compile_expr_range(start, end, kind),

            Expr::WindowExpr(window_expr, _) => self.compile_expr_window(window_expr),
            Expr::SimulationCall { .. } => Err(shape_ast::error::ShapeError::RuntimeError {
                message: "Simulation calls not supported".to_string(),
                location: None,
            }),

            // FromQuery should have been desugared before compilation
            Expr::FromQuery(_, _) => Err(shape_ast::error::ShapeError::RuntimeError {
                message: "FromQuery expressions must be desugared before compilation".to_string(),
                location: None,
            }),

            // Struct literal: TypeName { field: value, ... }
            Expr::StructLiteral {
                type_name,
                fields,
                span,
            } => self.compile_struct_literal(type_name, fields, *span),

            // Await expression: compile inner expr, emit Await opcode
            Expr::Await(inner, _span) => {
                if self.current_function.is_some() && !self.current_function_is_async {
                    return Err(shape_ast::error::ShapeError::SemanticError {
                        message: "'await' can only be used inside an async function".to_string(),
                        location: None,
                    });
                }
                if let Expr::Annotated {
                    annotation,
                    target,
                    span,
                } = inner.as_ref()
                {
                    self.compile_annotated_await_expr(annotation, target, *span)?;
                } else {
                    self.compile_expr(inner)?;
                    self.emit(Instruction::simple(OpCode::Await));
                    self.stamp_awaited_future_payload_type(inner);
                }
                Ok(())
            }

            // Join expression: await join all|race|any|settle { branch1, branch2, ... }
            // Note: Expr::Join is always wrapped in Expr::Await by the parser
            Expr::Join(join_expr, _span) => self.compile_join_expr(join_expr),

            // Annotated expression: @annotation expr
            Expr::Annotated {
                annotation,
                target,
                span,
            } => self.compile_annotated_expr(annotation, target, *span, None),

            // Async let: spawn task and bind future to local variable
            Expr::AsyncLet(async_let, _) => self.compile_async_let(async_let),

            // Async scope: structured concurrency boundary
            Expr::AsyncScope(inner, _) => self.compile_async_scope(inner),

            // Comptime blocks: execute at compile time, emit result as a constant
            Expr::Comptime(stmts, span) => {
                let extensions: Vec<_> = self
                    .extension_registry
                    .as_ref()
                    .map(|r| r.as_ref().clone())
                    .unwrap_or_default();
                let trait_impls = self.type_inference.env.trait_impl_keys();
                let known_type_symbols: std::collections::HashSet<String> = self
                    .struct_types
                    .keys()
                    .chain(self.type_aliases.keys())
                    .cloned()
                    .collect();
                let comptime_helpers = self.collect_comptime_helpers();
                // ADR-009 §4.1 (S2): comptime expressions consume the
                // per-compilation-unit freeze handle; the enclosing generic
                // function's type parameters enter via the scoped overlay
                // (`comptime_freeze_overlay` discovers `current_function`).
                // No per-site rebuild; a site without a handle is a compile
                // error (row 3).
                let freeze = self.comptime_freeze_overlay()?;
                // J-CT.2 — comptime-context items: trait defs, struct defs,
                // and comptime impl blocks are prepended to the mini-VM
                // program so `instance.method()` inside the comptime block
                // resolves via the standard UFCS / `Type::method` path
                // (audit §2.D carve-out).
                let comptime_impl_blocks = self.comptime_impl_blocks.clone();
                let comptime_context_trait_defs: Vec<_> =
                    self.trait_defs.values().cloned().collect();
                let comptime_context_struct_defs: Vec<_> = self
                    .comptime_context_struct_defs
                    .values()
                    .cloned()
                    .collect();
                let execution = super::comptime::execute_comptime_with_context(
                    stmts,
                    &comptime_helpers,
                    &comptime_impl_blocks,
                    &comptime_context_trait_defs,
                    &comptime_context_struct_defs,
                    &extensions,
                    trait_impls,
                    known_type_symbols,
                    freeze,
                )
                .map_err(|e| self.build_comptime_failure(&e, *span, "a compile-time block"))?;
                // §4.4: re-emit any `warning()` output anchored at this block.
                self.surface_comptime_warnings(&execution.warnings, *span);
                // Comptime blocks can emit directives via direct syntax.
                // They are processed with no implicit target binding.
                // ADR-009 D1 (S2): the block is its own expansion site.
                let module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
                let expansion_site = self.comptime_block_expansion_site(*span, &module_path);
                self.process_comptime_directives(execution.directives, "", &expansion_site)
                    .map_err(|e| {
                        // ADR-009 D1 (S4): provenance-carrying generated-decl
                        // failures pass through with their location notes.
                        self.preserve_or_wrap_directive_failure(e, "Comptime block", *span)
                    })?;
                // ADR-009 B1 S4: value-DEEP lift wall — the shared
                // `runtime_lift_rejection` fires on every reachable
                // typed-object node (nested objects/arrays, spellable model
                // forgeries), resolved against the mini-VM's own schema
                // registry so mini-VM-registered ids can be NAMED instead of
                // silently swallowing to `Null` on the materialization
                // fallback (scout risk 4 bypass channel).
                if let Some(message) = super::comptime::comptime_result_lift_rejection(
                    &execution.value,
                    &execution.schema_registry,
                ) {
                    return Err(shape_ast::error::ShapeError::SemanticError {
                        message: message.to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                // Convert the result to an expression and compile it.
                // Use nb_to_expr for complex types (arrays, objects) that
                // cannot be represented as a single literal.
                if let Ok(expr) = super::comptime::nb_to_expr_public(&execution.value, *span) {
                    self.compile_expr(&expr)?;
                } else {
                    let lit = super::comptime::vmvalue_to_literal(&execution.value);
                    self.compile_literal(&lit)?;
                }
                self.last_expr_schema = None;
                Ok(())
            }

            // Comptime for: evaluate iterable at compile time, unroll body for each element.
            Expr::ComptimeFor(cf, span) => self.compile_comptime_for(cf, *span),

            // Reference expression (&var / &mut var) - create a reference to a local variable.
            // Valid both as function arguments and standalone expressions (e.g., `let r = &x`).
            Expr::Reference {
                expr: inner,
                is_mutable,
                span,
            } => {
                let mode = if *is_mutable {
                    BorrowMode::Exclusive
                } else {
                    BorrowMode::Shared
                };
                let result = self.compile_reference_expr(inner, *span, mode).map(|_| ());
                if result.is_ok() {
                    self.set_last_expr_reference_result(mode, false);
                }
                result
            }

            // Table row literals — compiled via compile_table_rows() in the VariableDecl handler.
            // If we reach here, it means TableRows appeared outside a let binding context.
            Expr::TableRows(_, span) => Err(ShapeError::SemanticError {
                message: "table row literal `[...], [...]` can only be used as a variable initializer with a `Table<T>` type annotation".to_string(),
                location: Some(self.span_to_source_location(*span)),
            }),
        };

        if result.is_ok() {
            self.auto_deref_last_expr_result_if_needed()?;
        }
        self.current_expr_result_mode = saved_mode;
        result
    }

    /// Infer the type of an expression using the type inference engine
    ///
    /// Used for match exhaustiveness checking and other type-based validations.
    ///
    /// R5.3B: for `Expr::Identifier`, the compiler-owned `type_tracker`
    /// holds the authoritative type_name for let-locals, typed function
    /// parameters, and module bindings. The `type_inference` engine does
    /// not define those bindings in its environment, so it returns
    /// `UndefinedVariable` for the same identifiers. Consulting the tracker
    /// first preserves the temporal display name
    /// (`"DateTime"` / `"Duration"` / `"TimeSpan"`) through identifier
    /// resolution so the retarget guards at `binary_ops.rs:750-771` (Add)
    /// and `:1049-1072` (Sub) fire uniformly. For non-temporal identifiers
    /// the tracker value is equally valid (it matches the declared or
    /// inferred type), but we scope the tracker short-circuit narrowly to
    /// temporal names to avoid changing any existing non-temporal
    /// `infer_expr_type` behavior.
    pub(super) fn pending_empty_array_accumulator_name_for_expr<'a>(
        &self,
        expr: &'a Expr,
    ) -> Option<&'a str> {
        match expr {
            Expr::PropertyAccess { object, .. } => {
                self.pending_empty_array_accumulator_name_for_expr(object)
            }
            Expr::IndexAccess { object, .. } => {
                self.empty_array_accumulator_root_name_for_expr(object)
            }
            _ => None,
        }
    }

    fn empty_array_accumulator_root_name_for_expr<'a>(&self, expr: &'a Expr) -> Option<&'a str> {
        match expr {
            Expr::IndexAccess { object, .. } => {
                self.empty_array_accumulator_root_name_for_expr(object)
            }
            Expr::Identifier(name, _) if self.is_pending_empty_array_accumulator_name(name) => {
                Some(name.as_str())
            }
            _ => None,
        }
    }

    fn is_pending_empty_array_accumulator_name(&self, name: &str) -> bool {
        if let Some(local_idx) = self.resolve_local(name) {
            return self
                .empty_array_accumulators
                .contains_key(&crate::compiler::EmptyArrayAccumulatorKey::Local(local_idx))
                || self
                    .current_function_local_concrete_facts
                    .get(&local_idx)
                    .is_some_and(|fact| {
                        matches!(
                            fact.source,
                            crate::compiler::BindingConcreteFactSource::EmptyArrayAccumulator
                        )
                    });
        }
        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        self.module_bindings
            .get(&scoped_name)
            .is_some_and(|binding_idx| {
                self.empty_array_accumulators.contains_key(
                    &crate::compiler::EmptyArrayAccumulatorKey::ModuleBinding(*binding_idx),
                ) || self
                    .module_binding_concrete_facts
                    .get(binding_idx)
                    .is_some_and(|fact| {
                        matches!(
                            fact.source,
                            crate::compiler::BindingConcreteFactSource::EmptyArrayAccumulator
                        )
                    })
            })
    }

    pub(super) fn infer_expr_type(
        &mut self,
        expr: &Expr,
    ) -> Result<shape_runtime::type_system::Type> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        if let Expr::PropertyAccess { .. } = expr {
            if let Some(name) = self.pending_empty_array_accumulator_name_for_expr(expr) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer the type of field read from `{name}` because it was \
                         created from an unannotated empty array (`[]`); annotate the array \
                         (`let mut {name}: Array<T> = []`) before reading element fields"
                    ),
                    location: Some(
                        self.span_to_source_location(shape_ast::ast::Spanned::span(expr)),
                    ),
                });
            }
        }

        // T1 KEYSTONE (strict-flip, 2026-06-22): consult the inference engine's
        // POST-SOLVE per-expression type table FIRST, before the per-context
        // patch ladder below. The engine walked the FULL program — including
        // function bodies the module-scope `infer_expr` re-run at line ~2050
        // cannot see — and recorded the resolved type of every expression keyed
        // by its source span. This is the ROOT fix for the recurring
        // static-type-erasure class (`roster.map(|e| e.salary)` then `for v in
        // sals { if v > mx }`; `match m.get(k) { Some(n) => n + 1 }`): the
        // result type of a collection-dispatch / match-arm local reaches the use
        // site directly instead of erasing to `unknown`.
        //
        // The table holds ONLY fully-resolved types (`finalize_expr_type_table`
        // drops any entry that stayed a free variable post-solve), so a hit is a
        // genuine proof — never an Unknown-default. A miss falls through to the
        // existing per-context patches (kept as FALLBACK per surface-and-stop
        // discipline; retiring them is a separate cleanup stage).
        //
        // U4-1: `PropertyAccess` is NO LONGER excluded from the table consult.
        // The engine (post-U4-0) records every resolvable field read — including
        // closure-body and derived-object field reads — into the span table, so
        // a resolvable `rs[0].n` / `self.count` / `p.salary` hits the table
        // directly here. The previous PropertyAccess exclusion + the field-read
        // re-derivation ladder arms (#13/#14) that it guarded are DELETED:
        // STAGE-F1 (the engine's `TypeError::ConstraintViolation` in
        // `inference/access.rs`) is now the SOLE field-read strictness gate. An
        // un-annotatable field read (`f1`: `rs[0].n` where `rs`'s element type is
        // known only from a `push` into an unannotated `[]`) is rejected by the
        // engine BEFORE finalization, so it never enters the table — a genuine
        // miss here, which surfaces as the strict compile error. Nothing to mask.
        {
            let span = shape_ast::ast::Spanned::span(expr);
            if !span.is_dummy() {
                if let Some(resolved) = self.resolved_expr_types.get(&span) {
                    // A reference (`&T` / `&mut T`) binding is read THROUGH the
                    // reference in value position — the bytecode loads it via
                    // `DerefLoad`, and the GapA referent-projection patch below
                    // (`reference_referent_scalar_type_name`) supplies the
                    // projected `T`. Serving the raw `&T` here would route `r + 1` through
                    // a `&int + int` operand mismatch and break the auto-deref
                    // (borrow_refs `operator_deref::*` / `ref_dispatch::*`).
                    // Fall through to the patch ladder for a reference-typed
                    // result so the projection runs.
                    let is_reference =
                        matches!(resolved, Type::Concrete(TypeAnnotation::Borrow { .. }));
                    // FORWARD BINDER (U4-0 skeptic): a generic `|p: T| p.field`
                    // records a `Basic("unknown")` sentinel that is structurally
                    // a `Type::Concrete`, so `type_is_fully_resolved` keeps it and
                    // it SURVIVES finalization — it is NOT a table MISS. But it is
                    // not a real concrete type: consuming it would route the
                    // operand through as the type literally named "unknown".
                    // Treat the unknown-sentinel as un-inferable (fall through to
                    // the genuine surface-and-stop) rather than serving it.
                    // FORWARD BINDER parity with the engine (operators.rs:526):
                    // `Type::to_annotation()` lowers a lost TypeVar to the
                    // `"unknown"` sentinel as either `Basic("unknown")` OR
                    // `Reference("unknown")`. Match both via `as_type_name_str`
                    // so a `Reference`-shaped sentinel is also treated as a MISS.
                    let is_unknown_sentinel = matches!(
                        resolved,
                        Type::Concrete(ann) if ann.as_type_name_str() == Some("unknown")
                    );
                    if !is_reference && !is_unknown_sentinel {
                        return Ok(resolved.clone());
                    }
                }
            }
        }

        if let Expr::Identifier(name, _) = expr {
            // ADR-006 §2.7.30 (GapA, sibling of the `-> &T` call deref below):
            // an identifier bound to a reference value (`let r = &n`, or a
            // `&mut` of a local) is read THROUGH the reference in value position
            // — the bytecode loads it via `DerefLoad`. The reference binding's
            // tracker entry carries no scalar type_name, so the strict-typing
            // operand check (`r + 1`) would otherwise see `unknown`. Consult the
            // referent type recorded at bind time (the structural
            // `reference_value_*_referent_concrete_type` carrier, projected to a
            // scalar name by `reference_referent_scalar_type_name`; populated in
            // `finish_reference_binding_from_expr`) and forward it verbatim,
            // mirroring the already-auto-derefing method dispatch (`r.len()`).
            // Scoped to reference-BOUND identifiers (`let r = &n`): a
            // reference-TYPED param (`x: &int`) records no referent here and
            // stays a clean R4 compile-reject (`reference_typed_operand_span` in
            // `binary_ops.rs`). NOT a numeric coercion: `&int` -> `int`.
            // U4-5b: served from the one structural ConcreteType carrier — the
            // parallel referent display-string carrier is deleted.
            if let Some(referent) = self.reference_referent_scalar_type_name(name) {
                return Ok(Type::Concrete(TypeAnnotation::Basic(referent)));
            }
            if let Some(type_name) = self.tracker_type_name_for_identifier(name) {
                if matches!(type_name.as_str(), "DateTime" | "Duration" | "TimeSpan") {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(type_name)));
                }
                // Strict-typing-sweep: trust the type tracker for any
                // primitive scalar name. The runtime inference engine
                // ran on the original program AST and doesn't see
                // function-body `let a: u32 = 42` declarations, so
                // identifier inference returns Variable for those. The
                // tracker, in contrast, sees the annotation when
                // `compile_function_body` propagates declared types
                // into local slots. Falling back to it for primitive
                // names plugs the strict-typing hole that previously
                // routed through the deleted *Dynamic* shim.
                if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(&type_name)
                    || shape_runtime::type_system::BuiltinTypes::is_number_type_name(&type_name)
                    || matches!(type_name.as_str(), "bool" | "string" | "decimal" | "bigint")
                {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(type_name)));
                }
            }
            // U4-5: array-shaped identifier recovery, STRUCTURAL. The runtime
            // inference engine returns `Variable` (→ `unknown`) for function-body
            // `let xs: Array<T>` locals because it never saw the body declaration;
            // the compiler did, and recorded the full `ConcreteType::Array(elem)`
            // as an explicit binding fact. Read it back through
            // `identifier_concrete_type` so a downstream `xs + [..]` resolves to
            // the array shape and routes to `ArrayConcat` instead of erroring as
            // `unknown + T[]`. Replaces the deleted `type_name.strip_suffix("[]")`
            // re-parse (the read half of the Rep-B string round-trip).
            if let Some(shape_value::v2::ConcreteType::Array(elem)) =
                crate::compiler::monomorphization::type_resolution::identifier_concrete_type_pub(
                    self, name,
                )
            {
                if let Some(elem_ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&elem)
                {
                    return Ok(Type::Concrete(TypeAnnotation::Array(Box::new(elem_ann))));
                }
            }
            // R3-elemerasure (strict-flip): a `let x = a.first()` (scalar
            // element-returning builtin method) records the result
            // `ConcreteType` as an explicit binding fact (via the let-binding
            // `concrete_type_for_expr` recording, now element-aware), but its
            // tracker `type_name` stays `Unknown` because the
            // method-call compile path doesn't stamp `last_expr_numeric_type`
            // for the receiver-derived element type. Consult the recorded
            // ConcreteType so a downstream `x + 1` / `x == y` resolves the
            // operand. Scalar ConcreteTypes only — a composite result keeps the
            // existing array/schema side-table paths. The recorded ConcreteType
            // IS the proof (per ADR-006 §2.7.5); absent recording yields the
            // engine fallthrough below (no fabrication).
            let recorded_ct =
                crate::compiler::monomorphization::type_resolution::identifier_concrete_type_pub(
                    self, name,
                );
            if let Some(ct) = recorded_ct {
                // An array-typed local (`let xs: Array<T>`) reads back as
                // `Array<T>` in value position. The recorded element
                // ConcreteType IS the compile-time proof (ADR-006 §2.7.5); the
                // runtime inference engine returns `unknown` for body-local
                // array declarations it never saw. Surfacing the array shape
                // here lets a downstream `xs + [..]` route to `ArrayConcat`
                // (book idiom `weekdays = weekdays + [elem]`,
                // datetime.mdx §Date Range Iteration) instead of erroring as
                // `unknown + T[]`. Use the canonical `Array(_)` annotation
                // (not the `Vec<_>` generic render) so `type_display_name`
                // produces the `T[]` form that the ArrayConcat dispatch keys on.
                if let shape_value::v2::ConcreteType::Array(inner_ct) = &ct {
                    if let Some(inner_ann) =
                        crate::compiler::expressions::closures::concrete_type_to_type_annotation(
                            inner_ct,
                        )
                    {
                        return Ok(Type::Concrete(TypeAnnotation::Array(Box::new(inner_ann))));
                    }
                }
                if let Some(ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                {
                    if matches!(&ann, TypeAnnotation::Basic(_)) {
                        return Ok(Type::Concrete(ann));
                    }
                }
            }
        }

        // Phase 3e: function call return type from the tracker. The
        // runtime type-inference engine doesn't always see freshly
        // declared user functions; the tracker's
        // `function_return_concrete_types` is populated by the inference
        // pre-pass (`infer_return_concrete_types_from_types`) and serves
        // as the authoritative STRUCTURAL source for inferred return types
        // in the compiler's strict-typing decisions.
        if let Expr::FunctionCall { name, args, .. } = expr {
            // ADR-006 §2.7.30 (GapA): a `-> &T` callee's result is read THROUGH
            // the reference in value position (where `infer_expr_type` is asked —
            // binop operands, comparison sides). Project the declared `&T` return
            // to its referent `T` so the strict-typing operand check sees `T`,
            // mirroring the bytecode-side `DerefLoad`. NOT a numeric coercion: the
            // inner annotation is forwarded verbatim (`&int` -> `int`).
            if let Some(def) = self.function_defs.get(name) {
                if let Some(TypeAnnotation::Borrow { inner, .. }) = def.return_type.as_ref() {
                    if self.resolve_local(name).is_none() {
                        return Ok(Type::Concrete((**inner).clone()));
                    }
                }
            }
            // U4-4: declared return type of a (possibly IMPORTED) free function.
            // An imported `area` is registered in `function_defs` under its
            // QUALIFIED name (`calc::numbers::area`), so `function_defs.get(name)`
            // (bare) and the keystone span-table both miss. `find_function`
            // resolves the bare call to the compiled function index, whose
            // `.name` is the qualified key into `function_defs`. This is the
            // call-result type the deleted `last_expr_numeric_type` register
            // held after the imported call compiled — now sourced from the one
            // Type model. Scoped to a non-local, non-`-> &T` free function.
            if self.resolve_local(name).is_none() {
                if let Some(func_idx) = self.find_function(name) {
                    let qualified = self.program.functions[func_idx].name.clone();
                    if let Some(def) = self.function_defs.get(&qualified) {
                        // Skip GENERIC functions: a `fn clamp<T: Ord>(..) -> T`
                        // declares its return type as the unresolved type
                        // parameter `T` — serving that here would poison
                        // call-site type-argument inference / monomorphization.
                        // The concrete return type of a generic call is resolved
                        // by the monomorphizer, not this declared-return lookup.
                        let is_generic = def.type_params.as_ref().is_some_and(|tp| !tp.is_empty());
                        if !is_generic {
                            if let Some(ret) = def.return_type.as_ref() {
                                if !matches!(ret, TypeAnnotation::Borrow { .. }) {
                                    return Ok(Type::Concrete(ret.clone()));
                                }
                            }
                        }
                    }
                }
            }
            // U4-5b: inferred return type, served STRUCTURALLY from the
            // function's recorded return `ConcreteType` (declared returns are
            // served above from `function_defs`/`find_function`). The
            // ConcreteType is projected back to an inference `Type` at the use
            // site — no `"int"`/`"Vec<int>"` display-string round-trip.
            if let Some(ct) = self.type_tracker.get_function_return_concrete_type(name) {
                if let Some(ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)
                {
                    return Ok(Type::Concrete(ann));
                }
            }
            // U4-4: builtin / `__intrinsic_*` math-function return TYPE. These
            // are compiler-internal builtins whose bodies the inference engine
            // never walks (`environment/mod.rs:1269` — intrinsic type
            // registrations deliberately removed), so the span table has no
            // entry for `__intrinsic_max(series)`. The deleted
            // `last_expr_numeric_type` register was previously fed by a
            // hardcoded builtin-return-NumericType table; that knowledge now
            // feeds the ONE Type model here as a proper return `Type`, so
            // `numeric_type_of` derives the same kind from it. (`floor(3.7) + 1`,
            // `__intrinsic_max(s) - __intrinsic_min(s)`.)
            if self.resolve_local(name).is_none()
                && self.function_defs.get(name).is_none()
                && let Some(ty) = builtin_function_return_type(name)
            {
                return Ok(ty);
            }
            // U4-6 callable-return deletion: local/module callable binding
            // calls derive their return type from the binding's canonical
            // `InferenceFacts::binding_type` or retained closure body peek.
            // There is no slot-indexed return-name table to drift.
            if let Some(return_ty) = self.callable_binding_return_type(name, Some(args.len())) {
                return Ok(return_ty);
            }
        }
        if let Expr::QualifiedFunctionCall {
            namespace,
            function,
            ..
        } = expr
        {
            let local_qualified = format!("{}::{}", namespace, function);
            let mut candidates = Vec::with_capacity(2);
            if let Some(canonical) = self.resolve_canonical_module_path(namespace) {
                candidates.push(format!("{}::{}", canonical, function));
            }
            candidates.push(local_qualified);

            for call_name in candidates {
                if let Some(def) = self.function_defs.get(&call_name) {
                    let is_generic = def.type_params.as_ref().is_some_and(|tp| !tp.is_empty());
                    if !is_generic && let Some(ret) = def.return_type.as_ref() {
                        if let TypeAnnotation::Borrow { inner, .. } = ret {
                            return Ok(Type::Concrete((**inner).clone()));
                        }
                        return Ok(Type::Concrete(ret.clone()));
                    }
                }
                if let Some(ct) = self
                    .type_tracker
                    .get_function_return_concrete_type(&call_name)
                    && let Some(ann) =
                        crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)
                {
                    return Ok(Type::Concrete(ann));
                }
            }

            // WF-3A-tail: a native module builtin (`time::millis()`) is not a
            // monomorphized `function_defs` entry and has no tracker return
            // ConcreteType, so both candidate lookups above miss and inference
            // would fall through to the engine's fresh-var QualifiedFunctionCall
            // arm — erasing a bare `time::millis()` operand to `unknown`. Recover
            // the DECLARED scalar return type from the native module schema so
            // the call infers its true type in operand position. Scalar-only:
            // `Result<..>`/`Option<..>`/heap returns miss here and keep the
            // existing path (json::parse navigation unaffected). No fabrication.
            let canonical = self
                .resolve_canonical_module_path(namespace)
                .unwrap_or_else(|| namespace.clone());
            if let Some(ty) = self.native_module_declared_scalar_return_type(&canonical, function) {
                return Ok(ty);
            }
        }

        // D-β string-join receiver-kind fix (v0.3 KC #6(d), 2026-05-22):
        // `.toString()` / `.to_string()` always returns `string` (universal
        // method per `MethodTable::register_builtin_methods`). The runtime
        // inference engine's `Expr::MethodCall` arm falls through to the
        // method registry on `extract_receiver_info` `None` (Type::Variable
        // receivers e.g. `self[i]` on `Vec<T>` where the index access pushes
        // an `Indexable` constraint and returns a fresh type-var); the
        // sibling fix at `MethodTable::resolve_method_call` handles that
        // path, but `BytecodeCompiler::infer_expr_type` is called by the
        // strict-typing binop emitter at `binary_ops.rs:166-173` with a
        // `compiler.infer_expr_type` shape that does NOT go through the
        // `TypeInferenceEngine.infer_expr` constraint solver until the
        // fallback at line ~1570 below — and the toString MethodCall here
        // can short-circuit cleanly without needing the full engine. Per
        // ADR-006 §2.7.5 stamp-at-compile-time: the universal method's
        // return type is statically known at registration.
        if let Expr::MethodCall { method, .. } = expr {
            if method == "to_string" || method == "toString" {
                return Ok(shape_runtime::type_system::Type::Concrete(
                    TypeAnnotation::Basic("string".to_string()),
                ));
            }
        }

        // STAGE-S5 (string-method return-type recovery). The built-in
        // string-returning string methods (`charAt`, `slice`, `substring`,
        // `toUpperCase`, ... — the book strings.mdx §Methods set, typed
        // `... -> string` in `MethodTable::register_builtin_methods`) are not
        // monomorphized stdlib functions, so the module-scope inference engine
        // below has no binding for them and resolves `s.charAt(0)` /
        // `s.slice(1,3)` to `unknown`. That broke strict-typed downstream uses
        // exactly like the STAGE-S1 `s[i]` gap did before its fix: `s.charAt(0)
        // + "!"` rejected as `string + unknown`, and `print(s.charAt(0) ==
        // "h")` corrupted the heap (the `==` operand stayed unproven so no
        // typed `EqString` was emitted). The STAGE-S4 char model makes a single
        // character a real 1-char `string`, so `s.charAt(i)` MUST infer
        // `string` — exact parity with the `s[i]` arm below. Prove the receiver
        // is a `string` from its OWN resolved type (reading the receiver's
        // proof, not fabricating); the method's registered return type is
        // statically known per ADR-006 §2.7.5. A non-string receiver falls
        // through to the engine — no fabrication.
        if let Expr::MethodCall {
            receiver, method, ..
        } = expr
        {
            let returns_string = matches!(
                method.as_str(),
                "charAt"
                    | "slice"
                    | "substring"
                    | "toUpperCase"
                    | "toLowerCase"
                    | "trim"
                    | "trimStart"
                    | "trimEnd"
                    | "to_upper_case"
                    | "to_lower_case"
                    | "trim_start"
                    | "trim_end"
                    | "replace"
                    | "padStart"
                    | "padEnd"
                    | "repeat"
                    | "reverse"
            );
            if returns_string
                && matches!(
                    self.infer_expr_type(receiver),
                    Ok(Type::Concrete(TypeAnnotation::Basic(ref n))) if n == "string"
                )
            {
                return Ok(Type::Concrete(TypeAnnotation::Basic("string".to_string())));
            }
        }

        // U4-6 Tier 2: callable-array-element invocation. The parser models
        // `arr[i](args...)` as `MethodCall { method: "__call__", receiver:
        // IndexAccess { object: Identifier(arr), .. }, .. }`. Derive the
        // return type from the active `InferenceFacts` binding type
        // (`Array<Function<...>>`) instead of the deleted per-slot string map.
        if let Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } = expr
        {
            if method == "__call__" {
                if let Expr::IndexAccess { object, .. } = receiver.as_ref() {
                    if let Expr::Identifier(arr_name, _) = object.as_ref() {
                        if let Some(return_ty) =
                            self.indexed_callable_array_return_type(arr_name, Some(args.len()))
                        {
                            return Ok(return_ty);
                        }
                    }
                }
            }
        }

        // Phase 3e: BinaryOp Add of string-typed operands yields a string.
        // The runtime type-inference engine doesn't know about let-mut
        // accumulator types from the tracker, so chained concats like
        // `result + name + " "` would otherwise resolve to Unknown for
        // any inner sub-expression that isn't a bare identifier.
        if let Expr::BinaryOp {
            op: shape_ast::ast::BinaryOp::Add,
            left,
            right,
            ..
        } = expr
        {
            let lt = self.infer_expr_type(left).ok();
            let rt = self.infer_expr_type(right).ok();
            let is_string = |t: &Option<shape_runtime::type_system::Type>| {
                matches!(
                    t,
                    Some(shape_runtime::type_system::Type::Concrete(
                        TypeAnnotation::Basic(n)
                    )) if n == "string" || n == "char"
                )
            };
            if is_string(&lt) && is_string(&rt) {
                return Ok(Type::Concrete(TypeAnnotation::Basic("string".to_string())));
            }
        }

        // U4-1: ladder arms #13/#14 (PropertyAccess field-read re-derivation)
        // DELETED. They re-derived field-result types two ways — the schema-id /
        // object-field-contract lookup (arm #13) and the derived-object
        // `concrete_type_for_expr` projection (arm #14) — to recover a field type
        // the span-table consult above was deliberately skipping. With the
        // PropertyAccess exclusion removed, every resolvable field read hits
        // `resolved_expr_types` at the top of this function; an un-annotatable one
        // is a genuine miss and surfaces the engine's STAGE-F1 strictness error
        // (the SOLE field-read gate). No structural re-derivation remains.

        // WS-9: element type of `arr[i]` for a tracked-array receiver.
        //
        // The runtime inference engine the compiler shares here is at module
        // scope — it has no per-function parameter bindings, so
        // `infer_expr(IndexAccess { object: Identifier(param), .. })` returns
        // a disconnected, unresolved variable ("unknown"). That severs the
        // element type for an indexed unannotated parameter (`a[0] + b[0]`),
        // producing a spurious `unknown + unknown` reject.
        //
        // The program-wide inference pass DOES resolve the parameter — via
        // callsite unification of the concrete argument — and exposes its
        // Type through `InferenceFacts::function_signature`. `compile_function_body`
        // stamps the structural `ConcreteType::Array(elem)` onto the local slot
        // from that fact, so recovering the element type reads inference's own
        // output, not a string re-parse or fabricated kind. If inference could
        // not prove the parameter is an array this returns `None`, so the
        // operand stays unproven and the binop emitter raises a loud compile
        // error.
        // Tuple element access (book `fundamentals/variables` §Tuple Types):
        // a `[T0, T1, ...]`-annotated binding records a `ConcreteType::Tuple`
        // (via `declared_annotation_concrete_type`). `pair[k]` at a CONSTANT
        // integer index `k` resolves to the proven per-position element type so
        // a downstream binop (`pair[0] + pair[1]`) sees concrete operands. The
        // recorded ConcreteType IS the proof (ADR-006 §2.7.5); a non-constant
        // index or out-of-range `k` falls through (and the tuple-index type
        // checker in shape-runtime already rejects those at the inference pass).
        if let Expr::IndexAccess {
            object,
            index,
            end_index: None,
            ..
        } = expr
        {
            if let Expr::Identifier(obj_name, _) = object.as_ref() {
                let recorded_ct =
                    crate::compiler::monomorphization::type_resolution::identifier_concrete_type_pub(
                        self, obj_name,
                    );
                if let Some(shape_value::v2::ConcreteType::Tuple(elems)) = recorded_ct {
                    let k = match index.as_ref() {
                        Expr::Literal(shape_ast::ast::Literal::Int(i), _) => Some(*i),
                        Expr::Literal(shape_ast::ast::Literal::TypedInt(i, _), _) => Some(*i),
                        _ => None,
                    };
                    if let Some(k) = k {
                        if k >= 0 && (k as usize) < elems.len() {
                            let elem_ct = elems[k as usize].clone();
                            if let Some(ann) =
                                crate::compiler::expressions::closures::concrete_type_to_type_annotation(&elem_ct)
                            {
                                return Ok(Type::Concrete(ann));
                            }
                        }
                    }
                }
            }
        }

        if let Expr::IndexAccess {
            object,
            end_index: None,
            ..
        } = expr
        {
            // String index `s[i]` — the i-th character is a 1-char `string`
            // (STAGE-S4 char model: Shape has no first-class `char` type; the
            // VM `dispatch_get_prop` String arm + `s.charAt(i)` both produce a
            // real 1-char `NativeKind::String`). Prove the receiver is a
            // `string` from its own resolved type — reading the receiver's
            // proof, not fabricating — so a downstream strict-typed use
            // (`acc + s[i]`, `s[i] == "x"`) sees `string`, not `unknown`. Must
            // precede the array-element recovery below (string is not array-
            // shaped, so those would return None → `unknown`).
            if matches!(
                self.infer_expr_type(object),
                Ok(Type::Concrete(TypeAnnotation::Basic(ref n))) if n == "string"
            ) {
                return Ok(Type::Concrete(TypeAnnotation::Basic("string".to_string())));
            }
            if let Some(elem) = self.tracked_array_element_type(object) {
                return Ok(elem);
            }
            // Nested-index element recovery (v0.3.3 B4, references slice D2):
            // `m[r][c]` has an `object` that is itself an `IndexAccess`, so the
            // identifier-only `tracked_array_element_type` above returns None.
            // Fall to the structural `concrete_type_for_expr`, which unwraps one
            // `Array` layer per index op (here: `m[r]` resolves to
            // `Array<int>`, this access unwraps to `int`). The recovered
            // ConcreteType IS the proof (ADR-006 §2.7.5); a non-array object
            // ConcreteType yields None and the operand stays unproven (clean
            // compile error, no fabrication).
            if let Some(ct) =
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, expr,
                )
            {
                if let Some(ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                {
                    return Ok(Type::Concrete(ann));
                }
            }
        }

        // R3-elemerasure (strict-flip): the builtin (PHF) array methods that
        // return the receiver element type (`first`/`last`/`pop`/`find`) or the
        // receiver array itself (`sort`/`reverse`/`take`/`drop`/`slice`/`skip`/
        // `clone`/`unique`/`flatten`/`distinct`/`sortBy`/`concat`) are not
        // monomorphized stdlib functions; the shared module-scope inference
        // engine below has no per-function bindings, so `a.first() + 1` and
        // `a.first() == a.last()` saw the receiver as `unknown` and the binop
        // emitter raised a spurious strict-typing reject. Recover the result
        // `ConcreteType` from the receiver's PROVEN `ConcreteType`, driven by
        // the method's REGISTERED `GenericMethodSignature` return shape (same
        // proof source as the closure-param-hint chain). Returns the receiver's
        // own `Array<T>` for a `SelfType` method, the element `T` for a
        // `ReceiverParam(0)` method. An unproven receiver / non-array / other
        // return shape falls through to the engine below — no fabrication.
        if let Expr::MethodCall {
            receiver, method, ..
        } = expr
        {
            if let Some(result_ct) =
                crate::compiler::monomorphization::type_resolution::method_call_receiver_derived_concrete_type(
                    self, receiver, method,
                )
            {
                if let Some(ann) = crate::compiler::expressions::closures::concrete_type_to_type_annotation(&result_ct) {
                    return Ok(Type::Concrete(ann));
                }
            }
        }

        // ROOT-2 (strict-flip, 2026-06-18): an INLINE method-call result
        // (`d.hour() + 1`, `dt.is_weekday() && ...`) must surface its declared
        // return type to the strict-typing binop operand check WITHOUT an
        // intervening `let` reconciliation. The `let h = d.hour()` form already
        // works because the let-binding records the result `ConcreteType` (via
        // `concrete_type_for_expr`) and reconciles a 2nd inference pass; the
        // inline operand never gets that pass. `concrete_type_for_expr`'s
        // `Expr::MethodCall` arm carries the receiver-derived return tables
        // (DateTime instance methods -> int/bool/string/DateTime; monomorphized
        // stdlib-call substituted return; PHF first/last/sort/... ) — the SAME
        // proof source the let path consumes. Consult it here so the inline and
        // let forms resolve identically. The receiver's proven ConcreteType IS
        // the proof (ADR-006 §2.7.5); an opaque method falls through to the
        // engine (no fabrication). int and number do NOT unify — `d.hour()`
        // resolves to `int`, `sin(x)` to `number`; neither is coerced.
        if let Expr::MethodCall { .. } = expr {
            if let Some(ct) =
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, expr,
                )
            {
                if let Some(ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                {
                    return Ok(Type::Concrete(ann));
                }
            }
        }

        // ROOT-2 (strict-flip, 2026-06-18): an INLINE free-function-call result
        // (`sin(x) + 1.0`, `abs(n) - 1`) must surface its declared return type
        // to the strict-typing binop operand check, exactly like the inline
        // method-call arm above. The let form (`let s = sin(x); s + 1.0`) works
        // via the let-binding's recorded ConcreteType + reconciliation 2nd pass;
        // the inline operand lacks that pass. `concrete_type_for_expr`'s
        // `Expr::FunctionCall` arm reduces the callee's declared return
        // annotation (substituting generic args from the call-site argument
        // types) — the SAME proof the let path consumes. The callee's declared
        // return annotation IS the proof (ADR-006 §2.7.5); an opaque/foreign
        // callee falls through to the engine (no fabrication). int and number
        // stay strict (`sin` -> number; an `int`-returning callee -> int).
        // Runs AFTER the `function_return_types` / callable-binding lookups
        // above so a tracker-recorded return name still takes precedence.
        if let Expr::FunctionCall { .. } = expr {
            if let Some(ct) =
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, expr,
                )
            {
                if let Some(ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                {
                    return Ok(Type::Concrete(ann));
                }
            }
        }

        // Array literal: resolve the element type from the literal's own
        // elements via the compiler's structural `concrete_type_for_expr`
        // (which sees function-body locals + for-loop variables + f-string
        // elements the module-scope runtime engine cannot). Without this, a
        // `[f"day-{i}"]` / `[i]` literal inside a loop body infers `unknown`
        // and a `xs + [..]` ArrayConcat operand check rejects it as
        // `string[] + unknown`. Per ADR-006 §2.7.5 the resolved element
        // ConcreteType IS the proof; an unresolvable / heterogeneous literal
        // yields `None` and falls through to the engine (no fabrication —
        // genuinely-untyped literals stay a clean compile error downstream).
        if let Expr::Array(..) = expr {
            if let Some(shape_value::v2::ConcreteType::Array(inner_ct)) =
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, expr,
                )
            {
                if let Some(inner_ann) =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(
                        &inner_ct,
                    )
                {
                    return Ok(Type::Concrete(TypeAnnotation::Array(Box::new(inner_ann))));
                }
            }
        }

        // U4-3 KEYSTONE (strict-flip, 2026-06-23): the fallback re-derivation
        // engine is DELETED. There is now exactly ONE L3 inference authority —
        // the engine span-table (`resolved_expr_types`, consulted FIRST at the
        // top of this function) plus the per-context proof patches above. When
        // none of those proved a concrete type, the expression is genuinely
        // un-inferable: a span-table MISS is a LOUD surface-and-stop compile
        // error, NEVER a re-derivation.
        //
        // The deleted fallback (`self.type_inference.infer_expr(expr)`) re-ran
        // Engine B — the module-scope inference env, blind to function-body
        // locals — and papered over genuine erasure by returning `unknown` (or a
        // mis-derived module-scope type) for any body-local expression. That is
        // the dynamic-fallback shape CLAUDE.md §Forbidden Patterns refuses: it
        // masked real type-erasure bugs (the `f8`/`h1`/`h2`/`h4b` closure-field
        // class) behind a second, weaker engine. U4-3pre closed the last
        // engine-completeness gap (resilient span-table recording), so the
        // §5(A) zero-OK_RESOLVED-miss property holds and the fallback can go.
        //
        // NOTE: `self.type_inference` (the FIELD) survives — it still backs the
        // env LOOKUP sites (trait dispatch / enum + alias resolution) at
        // `binary_ops.rs`, `type_ops.rs`, `helpers.rs`, etc. Only the
        // `infer_expr` re-derivation CALL is gone.
        Err(shape_ast::error::ShapeError::SemanticError {
            message: format!(
                "Could not infer the type of this expression at compile time. \
                 Strict typing requires every expression to have a known \
                 concrete type; annotate the binding or value (e.g. \
                 `let x: T = ...`), or rewrite so the type is inferable. \
                 (expr span: {:?})",
                shape_ast::ast::Spanned::span(expr)
            ),
            location: None,
        })
    }

    /// WS-9: when `object` is an identifier tracked as a homogeneous array
    /// (`Array<T>` / `Vec<T>` / `T[]`), return the element type `T` as a
    /// concrete inference `Type`. Returns `None` for non-identifier
    /// receivers, untracked names, and non-array receivers.
    ///
    /// U4-5/U4-7: reads the element type STRUCTURALLY from the recorded
    /// `ConcreteType` (`identifier_concrete_type` consults whole-binding
    /// concrete tables), not by stripping a
    /// `"Vec<...>"`/`"int[]"` tracker string. The old `strip_prefix("Array<")`
    /// / `strip_suffix("[]")` re-parse (the read half of the Rep-B string
    /// round-trip whose write half is `tracked_type_name_from_annotation`)
    /// is deleted — the structural `Array(elem)` ConcreteType is the source.
    pub(super) fn tracked_array_element_type(
        &self,
        object: &Expr,
    ) -> Option<shape_runtime::type_system::Type> {
        use shape_runtime::type_system::Type;

        let Expr::Identifier(name, _) = object else {
            return None;
        };
        // RefDispatch (v0.3.3): `r[i]` on `let r = &a` (a: Array<T>). The
        // reference binding carries no array ConcreteType of its own; its
        // referent's array element ConcreteType was recorded at bind time
        // (`record_reference_referent_concrete_type`). Consult the referent's
        // ConcreteType so the element type is recovered THROUGH the reference,
        // mirroring the scalar value-position auto-deref (`r + 1`). A scalar
        // referent (`&int`) has no array shape and falls through to None.
        let ct = crate::compiler::monomorphization::type_resolution::identifier_concrete_type_pub(
            self, name,
        )
        .or_else(|| self.reference_referent_concrete_type(name))?;
        let shape_value::v2::ConcreteType::Array(elem) = ct else {
            return None;
        };
        let elem_ann =
            crate::compiler::expressions::closures::concrete_type_to_type_annotation(&elem)?;
        Some(Type::Concrete(elem_ann))
    }

    /// R5.3B helper: return the tracker-recorded `type_name` for an
    /// identifier, searching local slots first and falling back to module
    /// bindings. Returns `None` if the identifier is neither a local nor a
    /// module binding, or if the tracker has no type_name on that slot.
    pub(super) fn tracker_type_name_for_identifier(&self, name: &str) -> Option<String> {
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                if let Some(ref tn) = info.type_name {
                    return Some(tn.clone());
                }
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                if let Some(ref tn) = info.type_name {
                    return Some(tn.clone());
                }
            }
        }
        None
    }

    /// Phase 3d helper: look up a tracker-recorded schema_id for an
    /// expression. Currently handles the identifier case (locals + module
    /// bindings), which covers the `self.field` use case in trait method
    /// bodies.
    fn tracker_schema_id_for_expr(&mut self, expr: &Expr) -> Option<u32> {
        {
            let lookup_by_name = |tn: &str| -> Option<u32> {
                self.type_tracker
                    .schema_registry()
                    .get(tn)
                    .map(|s| s.id)
                    .or_else(|| {
                        // Phase 3e: fall back to module-scope-resolved name
                        // (e.g. `A` inside `mod m` resolves to `m::A`). The
                        // schema is registered under the qualified form;
                        // local/binding type_name often holds the bare form.
                        let qualified = self.resolve_type_name(tn);
                        if qualified != tn {
                            self.type_tracker
                                .schema_registry()
                                .get(&qualified)
                                .map(|s| s.id)
                        } else {
                            None
                        }
                    })
            };
            if let Expr::Identifier(name, _) = expr {
                if let Some(local_idx) = self.resolve_local(name) {
                    if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                        if let Some(id) = info.schema_id {
                            return Some(id);
                        }
                        if let Some(ref tn) = info.type_name {
                            if let Some(id) = lookup_by_name(tn) {
                                return Some(id);
                            }
                        }
                    }
                }
                if let Some(&binding_idx) = self.module_bindings.get(name) {
                    if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                        if let Some(id) = info.schema_id {
                            return Some(id);
                        }
                        if let Some(ref tn) = info.type_name {
                            if let Some(id) = lookup_by_name(tn) {
                                return Some(id);
                            }
                        }
                    }
                }
            }
        }
        // WS-9c: a direct `f(...).field` access — the receiver is a call to
        // an unannotated function whose inferred return type is an anonymous
        // object. Derive/register the return-object schema from the active
        // inference facts so the property access types without an intervening
        // `let`.
        if let Expr::FunctionCall { name, .. } = expr {
            if let Some(schema_id) = self.inferred_return_object_schema_id(name) {
                return Some(schema_id);
            }
        }
        None
    }
}
