//! ADR-009 C3 #14 (slice 1) — the G9 pseudo-tuple usage classifier: the SINGLE
//! traversal core for polymorphic template bodies.
//!
//! # The G9 ruling this enforces
//!
//! C3-G9 resolved the Args carrier as a SPECIALIZATION-RESOLVED PSEUDO-TUPLE:
//! `args[i]` / `args.length` are TEMPLATE-LEVEL constructs. No tuple value ever
//! exists at runtime — at specialization (stage S1c/S1d), a constant-index
//! `args[i]` resolves to the target's i-th typed parameter slot, `args.length`
//! resolves to a constant, and a mutation-return (`return args` / a final bare
//! `args`) specializes to a COMPILER-INTERNAL per-target aggregate at the weave
//! boundary (never user-visible; never the boxed `NewArray` path).
//!
//! # One walker (binding invariant — do not fork)
//!
//! This module is the single traversal core. Stage 4 (the specialization
//! rewrite) adds its rewrite face ON THIS SAME CORE — never a second, drifting
//! walker. The traversal mirrors the exhaustive Statement/Expr skeleton of
//! `monomorphization/substitution.rs` (the precedent full-AST walk): every
//! `match` is exhaustive with no catch-all arm, so a new AST variant is a
//! compile error here, exactly like the substitution walker.
//!
//! # Known validation boundaries (named, deliberate)
//!
//! - **Out-of-range constant indices are NOT construction-checkable** — arity
//!   is a property of the frozen TARGET, which construction never sees. The
//!   specialization stage (stage 4) rejects `args[7]` against a 2-parameter
//!   target with an application-site error naming both signatures.
//! - **Interpolated f-string contents are not scanned.** A `Literal::
//!   FormattedString` carries its interpolation as raw text until emission; a
//!   pseudo-tuple reference inside one (`f"{args}"`) is caught downstream by
//!   ordinary identifier resolution after the pseudo-tuple has resolved away
//!   (the name no longer exists), never silently honored.
//!
//! # The `__c3_` reserved prefix
//!
//! Stage 4 mints specialization-internal locals under the `__c3_` prefix. The
//! walker rejects ANY identifier carrying that prefix in a template body so a
//! minted name can never collide with (or capture) user spelling — the same
//! internal-name discipline as the legacy wrapper's `__args`/`__result`/`__ctx`
//! locals (`compile_annotation_wrapper`, `functions_annotations.rs:4232-4234`),
//! but enforced at construction instead of relied on by convention.

use shape_ast::ast::expr_helpers::{BlockItem, QueryClause};
use shape_ast::ast::expressions::{EnumConstructorPayload, Expr, ObjectEntry};
use shape_ast::ast::functions::{Annotation, FunctionParameter};
use shape_ast::ast::literals::Literal;
use shape_ast::ast::patterns::{DestructurePattern, Pattern, PatternConstructorFields};
use shape_ast::ast::program::{Assignment, VariableDecl};
use shape_ast::ast::statements::{ForInit, Statement};
use shape_ast::ast::types::{ExtendStatement, MethodDef, TypeAnnotation};
use shape_ast::ast::windows::{WindowExpr, WindowFunction};
use shape_ast::error::{Result, ShapeError};

/// The reserved specialization-internal identifier prefix (see module docs).
pub(in crate::compiler) const RESERVED_SPECIALIZATION_PREFIX: &str = "__c3_";

/// Validate every use of the pseudo-tuple parameter (`args_param`) and the
/// template type parameter (`type_param`) in a polymorphic BEFORE template
/// body.
///
/// Legal uses of `args_param`, exactly:
///
/// - `args[<int literal>]` in read position (`IndexAccess` with a
///   `Literal::Int` index and no slice end),
/// - the same shape as an assignment target (`args[<int literal>] = expr`),
/// - `args.length` (plain, non-optional property access),
/// - `return args` (statement form, and the parser's expression-position
///   `return` twin — `Expr::Return` — which is the same authored spelling
///   built by the block-item return path in the parser),
/// - a FINAL bare `args` expression statement at the top level of the body
///   (the implicit-return tail).
///
/// Everything else involving `args_param` or `type_param` is a NAMED
/// rejection with a positive twin (see the `reject_*` constructors); the
/// walker also rejects rebinding either name (a shadowed pseudo-tuple could
/// silently change what stage 4 rewrites) and any identifier with the
/// reserved `__c3_` prefix.
///
/// Called from `CheckedTemplateBuilder::finish()` for
/// `TemplateSig::PolymorphicArgs` only — concrete bodies and polymorphic
/// AFTER bodies (`result`) carry ordinary values with no pseudo-tuple
/// surface.
pub(in crate::compiler) fn validate_pseudo_tuple_uses(
    body: &[Statement],
    args_param: &str,
    type_param: &str,
) -> Result<()> {
    let scan = Scan {
        args_param,
        type_param,
    };
    let last = body.len().checked_sub(1);
    for (i, stmt) in body.iter().enumerate() {
        // The implicit-return tail: a FINAL bare `args` expression statement
        // at the TOP LEVEL of the body is the mutation-return spelling.
        if Some(i) == last {
            if let Statement::Expression(Expr::Identifier(name, _), _) = stmt {
                if name == args_param {
                    continue;
                }
            }
        }
        scan.statement(stmt, ScanMode::TemplateBody)?;
    }
    Ok(())
}

/// Where the walker currently is relative to the closure boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    /// Directly inside the template body — the pseudo-tuple surface is live.
    TemplateBody,
    /// Inside a closure sub-expression — the pseudo-tuple does not cross
    /// closure boundaries in S1, so ANY occurrence of either name rejects.
    ClosureInterior,
}

struct Scan<'a> {
    args_param: &'a str,
    type_param: &'a str,
}

fn reject(message: String) -> ShapeError {
    ShapeError::SemanticError {
        message,
        location: None,
    }
}

impl<'a> Scan<'a> {
    // ---------------------------------------------------------------------
    // Named rejections (uncoded sentences + positive twins; S5 owns C09xx
    // minting from C0931+ — no code brackets here).
    // ---------------------------------------------------------------------

    fn reject_non_constant_index(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple requires a compile-time-constant index: write \
             `{args}[<int literal>]` (for example `{args}[0]`), which resolves to the target's \
             parameter slot at specialization; a non-constant index has no parameter slot to \
             resolve to",
            args = self.args_param
        ))
    }

    fn reject_slice(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple cannot be sliced: no tuple value exists at runtime; use \
             `{args}[<int literal>]` for one typed parameter slot or `{args}.length` for the \
             parameter count",
            args = self.args_param
        ))
    }

    fn reject_other_property(&self, property: &str) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple has no property `{property}`: its only property is \
             `{args}.length` (a specialization-time constant); use `{args}[<int literal>]` for \
             the typed parameter slots",
            args = self.args_param
        ))
    }

    fn reject_optional_access(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple is never optional: write `{args}.length` as a plain \
             access; optional chaining (`?.`) has no null case to guard on a \
             specialization-resolved constant",
            args = self.args_param
        ))
    }

    fn reject_bare_value(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple has no first-class value: address one typed parameter \
             slot as `{args}[<int literal>]`, the parameter count as `{args}.length`, or return \
             the whole mutated pack with `return {args}` (or a final bare `{args}`)",
            args = self.args_param
        ))
    }

    fn reject_closure_occurrence(&self, name: &str) -> ShapeError {
        reject(format!(
            "`{name}` cannot appear inside a closure: the `{args}` pseudo-tuple is \
             specialization-resolved and does not cross closure boundaries in S1; do the \
             pseudo-tuple access in the template body itself and pass the resulting value into \
             the closure",
            args = self.args_param
        ))
    }

    fn reject_type_param_annotation(&self) -> ShapeError {
        reject(format!(
            "the template type parameter `{tp}` cannot appear in a body-internal type \
             annotation: it names the whole bound signature and resolves away at \
             specialization; annotate with a concrete type or let inference type the binding",
            tp = self.type_param
        ))
    }

    fn reject_reserved_prefix(&self, name: &str) -> ShapeError {
        reject(format!(
            "identifier `{name}` uses the reserved prefix `{prefix}` (the compiler-internal \
             namespace for specialization-minted locals); choose a name without the `{prefix}` \
             prefix",
            prefix = RESERVED_SPECIALIZATION_PREFIX
        ))
    }

    fn reject_rebind(&self, name: &str) -> ShapeError {
        reject(format!(
            "`{name}` cannot be rebound inside a template body: the name is part of the \
             pseudo-tuple surface (`{args}` / its type parameter `{tp}`) and rebinding would \
             shadow the specialization-resolved meaning; choose a different binding name",
            args = self.args_param,
            tp = self.type_param
        ))
    }

    // ---------------------------------------------------------------------
    // Name checks
    // ---------------------------------------------------------------------

    /// Any identifier spelling, in any role: the reserved-prefix check.
    fn check_reserved(&self, name: &str) -> Result<()> {
        if name.starts_with(RESERVED_SPECIALIZATION_PREFIX) {
            return Err(self.reject_reserved_prefix(name));
        }
        Ok(())
    }

    /// A name in BINDING position (let/for/match/query bindings).
    fn check_binding_name(&self, name: &str, mode: ScanMode) -> Result<()> {
        self.check_reserved(name)?;
        match mode {
            ScanMode::ClosureInterior => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_closure_occurrence(name));
                }
            }
            ScanMode::TemplateBody => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_rebind(name));
                }
            }
        }
        Ok(())
    }

    /// A name in ASSIGNMENT-target position (mutating an existing binding).
    fn check_assign_target_name(&self, name: &str, mode: ScanMode) -> Result<()> {
        self.check_reserved(name)?;
        match mode {
            ScanMode::ClosureInterior => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_closure_occurrence(name));
                }
            }
            ScanMode::TemplateBody => {
                // `args = e` mutates the whole pack as a value — not a legal
                // use (only per-slot `args[i] = e` is).
                if name == self.args_param {
                    return Err(self.reject_bare_value());
                }
            }
        }
        Ok(())
    }

    fn is_args_identifier(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Identifier(name, _) if name == self.args_param)
    }

    // ---------------------------------------------------------------------
    // Statements
    // ---------------------------------------------------------------------

    fn statements(&self, stmts: &[Statement], mode: ScanMode) -> Result<()> {
        for stmt in stmts {
            self.statement(stmt, mode)?;
        }
        Ok(())
    }

    fn statement(&self, stmt: &Statement, mode: ScanMode) -> Result<()> {
        match stmt {
            Statement::Return(value, _) => {
                // `return args` — the mutation-return spelling (template body
                // only; inside a closure it is an occurrence like any other).
                if mode == ScanMode::TemplateBody {
                    if let Some(inner) = value {
                        if self.is_args_identifier(inner) {
                            return Ok(());
                        }
                    }
                }
                self.opt_expr(value.as_ref(), mode)
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => Ok(()),
            Statement::VariableDecl(decl, _) => self.variable_decl(decl, mode),
            Statement::Assignment(assign, _) => self.assignment(assign, mode),
            Statement::Expression(expr, _) => self.expr(expr, mode),
            Statement::For(for_loop, _) => {
                match &for_loop.init {
                    ForInit::ForIn { pattern, iter } => {
                        self.destructure_pattern_binding(pattern, mode)?;
                        self.expr(iter, mode)?;
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.statement(init, mode)?;
                        self.expr(condition, mode)?;
                        self.expr(update, mode)?;
                    }
                }
                self.statements(&for_loop.body, mode)
            }
            Statement::While(while_loop, _) => {
                self.expr(&while_loop.condition, mode)?;
                self.statements(&while_loop.body, mode)
            }
            Statement::If(if_stmt, _) => {
                self.expr(&if_stmt.condition, mode)?;
                self.statements(&if_stmt.then_body, mode)?;
                if let Some(else_body) = &if_stmt.else_body {
                    self.statements(else_body, mode)?;
                }
                Ok(())
            }
            Statement::Extend(ext, _) => self.extend(ext, mode),
            Statement::SetParamType {
                type_annotation, ..
            } => self.type_annotation(type_annotation, mode),
            Statement::SetParamTypeExpr { expression, .. } => self.expr(expression, mode),
            Statement::SetParamValue { expression, .. } => self.expr(expression, mode),
            Statement::SetReturnType {
                type_annotation, ..
            } => self.type_annotation(type_annotation, mode),
            Statement::SetReturnExpr { expression, .. } => self.expr(expression, mode),
            Statement::ReplaceBody { body, .. } => self.statements(body, mode),
            Statement::ReplaceBodyExpr { expression, .. } => self.expr(expression, mode),
            Statement::ReplaceModuleExpr { expression, .. } => self.expr(expression, mode),
            Statement::ExtendItemsExpr { expression, .. } => self.expr(expression, mode),
        }
    }

    fn variable_decl(&self, decl: &VariableDecl, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_binding(&decl.pattern, mode)?;
        if let Some(annotation) = &decl.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(decl.value.as_ref(), mode)
    }

    fn assignment(&self, assign: &Assignment, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_assign_target(&assign.pattern, mode)?;
        self.expr(&assign.value, mode)
    }

    fn extend(&self, ext: &ExtendStatement, mode: ScanMode) -> Result<()> {
        for method in &ext.methods {
            self.method_def(method, mode)?;
        }
        Ok(())
    }

    fn method_def(&self, method: &MethodDef, mode: ScanMode) -> Result<()> {
        self.check_reserved(&method.name)?;
        for annotation in &method.annotations {
            self.annotation_args(annotation, mode)?;
        }
        for param in &method.params {
            self.function_parameter(param, mode)?;
        }
        if let Some(when) = &method.when_clause {
            self.expr(when, mode)?;
        }
        if let Some(ret) = &method.return_type {
            self.type_annotation(ret, mode)?;
        }
        self.statements(&method.body, mode)
    }

    fn function_parameter(&self, param: &FunctionParameter, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_binding(&param.pattern, mode)?;
        if let Some(annotation) = &param.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(param.default_value.as_ref(), mode)
    }

    fn annotation_args(&self, annotation: &Annotation, mode: ScanMode) -> Result<()> {
        for arg in &annotation.args {
            self.expr(arg, mode)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Patterns
    // ---------------------------------------------------------------------

    fn destructure_pattern_binding(&self, pat: &DestructurePattern, mode: ScanMode) -> Result<()> {
        match pat {
            DestructurePattern::Identifier(name, _) => self.check_binding_name(name, mode),
            DestructurePattern::Array(items) => {
                for item in items {
                    self.destructure_pattern_binding(item, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern_binding(&field.pattern, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern_binding(inner, mode),
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    self.check_binding_name(&binding.name, mode)?;
                    self.type_annotation(&binding.type_annotation, mode)?;
                }
                Ok(())
            }
        }
    }

    fn destructure_pattern_assign_target(
        &self,
        pat: &DestructurePattern,
        mode: ScanMode,
    ) -> Result<()> {
        match pat {
            DestructurePattern::Identifier(name, _) => self.check_assign_target_name(name, mode),
            DestructurePattern::Array(items) => {
                for item in items {
                    self.destructure_pattern_assign_target(item, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern_assign_target(&field.pattern, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern_assign_target(inner, mode),
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    self.check_assign_target_name(&binding.name, mode)?;
                    self.type_annotation(&binding.type_annotation, mode)?;
                }
                Ok(())
            }
        }
    }

    fn match_pattern(&self, pat: &Pattern, mode: ScanMode) -> Result<()> {
        match pat {
            Pattern::Identifier { name, .. } => self.check_binding_name(name, mode),
            Pattern::Typed {
                name,
                type_annotation,
                ..
            } => {
                self.check_binding_name(name, mode)?;
                self.type_annotation(type_annotation, mode)
            }
            Pattern::Literal(_) | Pattern::Wildcard => Ok(()),
            Pattern::Array(items) => {
                for item in items {
                    self.match_pattern(item, mode)?;
                }
                Ok(())
            }
            Pattern::Object(fields) => {
                for (_, field_pat) in fields {
                    self.match_pattern(field_pat, mode)?;
                }
                Ok(())
            }
            Pattern::Constructor { fields, .. } => match fields {
                PatternConstructorFields::Unit => Ok(()),
                PatternConstructorFields::Tuple(items) => {
                    for item in items {
                        self.match_pattern(item, mode)?;
                    }
                    Ok(())
                }
                PatternConstructorFields::Struct(entries) => {
                    for (_, field_pat) in entries {
                        self.match_pattern(field_pat, mode)?;
                    }
                    Ok(())
                }
            },
        }
    }

    // ---------------------------------------------------------------------
    // Type annotations
    // ---------------------------------------------------------------------

    /// A body-internal type annotation must not mention the template type
    /// parameter (it resolves away at specialization; there is no nameable
    /// type behind it).
    fn type_annotation(&self, annotation: &TypeAnnotation, mode: ScanMode) -> Result<()> {
        if self.annotation_mentions_type_param(annotation) {
            return Err(match mode {
                ScanMode::TemplateBody => self.reject_type_param_annotation(),
                ScanMode::ClosureInterior => self.reject_closure_occurrence(self.type_param),
            });
        }
        Ok(())
    }

    fn annotation_mentions_type_param(&self, annotation: &TypeAnnotation) -> bool {
        match annotation {
            TypeAnnotation::Basic(name) => name == self.type_param,
            TypeAnnotation::Array(inner) => self.annotation_mentions_type_param(inner),
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => items
                .iter()
                .any(|item| self.annotation_mentions_type_param(item)),
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| self.annotation_mentions_type_param(&field.type_annotation)),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| self.annotation_mentions_type_param(&param.type_annotation))
                    || self.annotation_mentions_type_param(returns)
            }
            TypeAnnotation::Generic { name, args } => {
                (!name.is_qualified() && name.name() == self.type_param)
                    || args.iter().any(|arg| self.annotation_mentions_type_param(arg))
            }
            TypeAnnotation::Reference(path) => {
                !path.is_qualified() && path.name() == self.type_param
            }
            TypeAnnotation::Borrow { inner, .. } => self.annotation_mentions_type_param(inner),
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => false,
            TypeAnnotation::Dyn(paths) => paths
                .iter()
                .any(|path| !path.is_qualified() && path.name() == self.type_param),
            TypeAnnotation::Existential { inner, .. } => {
                self.annotation_mentions_type_param(inner)
            }
        }
    }

    // ---------------------------------------------------------------------
    // Expressions
    // ---------------------------------------------------------------------

    fn opt_expr(&self, expr: Option<&Expr>, mode: ScanMode) -> Result<()> {
        match expr {
            Some(inner) => self.expr(inner, mode),
            None => Ok(()),
        }
    }

    fn exprs(&self, exprs: &[Expr], mode: ScanMode) -> Result<()> {
        for expr in exprs {
            self.expr(expr, mode)?;
        }
        Ok(())
    }

    fn named_exprs(&self, entries: &[(String, Expr)], mode: ScanMode) -> Result<()> {
        for (_, value) in entries {
            self.expr(value, mode)?;
        }
        Ok(())
    }

    /// The legal `args[<int literal>]` shape, checked when `object` is the
    /// pseudo-tuple identifier in `TemplateBody` mode. Returns the named
    /// rejection for slices and non-constant indices.
    fn args_index_access(&self, index: &Expr, end_index: Option<&Expr>) -> Result<()> {
        if end_index.is_some() {
            return Err(self.reject_slice());
        }
        match index {
            Expr::Literal(Literal::Int(_), _) => Ok(()),
            _ => Err(self.reject_non_constant_index()),
        }
    }

    fn expr(&self, expr: &Expr, mode: ScanMode) -> Result<()> {
        match expr {
            // Leaves. FormattedString interpolation is deliberately not
            // scanned (see the module docs' named boundary).
            Expr::Literal(_, _)
            | Expr::DataRef(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::Duration(_, _)
            | Expr::Continue(_)
            | Expr::Unit(_) => Ok(()),

            Expr::Identifier(name, _) => {
                self.check_reserved(name)?;
                match mode {
                    ScanMode::TemplateBody => {
                        if name == self.args_param {
                            // Bare `args` in a value position (the legal
                            // return/tail spellings are handled by the
                            // callers before recursion reaches here).
                            return Err(self.reject_bare_value());
                        }
                        Ok(())
                    }
                    ScanMode::ClosureInterior => {
                        if name == self.args_param || name == self.type_param {
                            return Err(self.reject_closure_occurrence(name));
                        }
                        Ok(())
                    }
                }
            }

            Expr::TypeSyntax(annotation, _) => self.type_annotation(annotation, mode),

            Expr::DataRelativeAccess { reference, .. } => self.expr(reference, mode),

            Expr::PropertyAccess {
                object,
                property,
                optional,
                span: _,
            } => {
                if mode == ScanMode::TemplateBody && self.is_args_identifier(object) {
                    if property != "length" {
                        return Err(self.reject_other_property(property));
                    }
                    if *optional {
                        return Err(self.reject_optional_access());
                    }
                    return Ok(());
                }
                self.expr(object, mode)
            }

            Expr::IndexAccess {
                object,
                index,
                end_index,
                span: _,
            } => {
                if mode == ScanMode::TemplateBody && self.is_args_identifier(object) {
                    return self.args_index_access(index, end_index.as_deref());
                }
                self.expr(object, mode)?;
                self.expr(index, mode)?;
                self.opt_expr(end_index.as_deref(), mode)
            }

            Expr::BinaryOp { left, right, .. } => {
                self.expr(left, mode)?;
                self.expr(right, mode)
            }

            Expr::FuzzyComparison { left, right, .. } => {
                self.expr(left, mode)?;
                self.expr(right, mode)
            }

            Expr::UnaryOp { operand, .. } => self.expr(operand, mode),

            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                span: _,
            } => {
                self.check_reserved(name)?;
                self.exprs(const_args, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::QualifiedFunctionCall {
                namespace,
                function,
                const_args,
                args,
                named_args,
                span: _,
            } => {
                self.check_reserved(namespace)?;
                self.check_reserved(function)?;
                self.exprs(const_args, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::EnumConstructor { payload, .. } => match payload {
                EnumConstructorPayload::Unit => Ok(()),
                EnumConstructorPayload::Tuple(items) => self.exprs(items, mode),
                EnumConstructorPayload::Struct(fields) => self.named_exprs(fields, mode),
            },

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                span: _,
            } => {
                self.expr(condition, mode)?;
                self.expr(then_expr, mode)?;
                self.opt_expr(else_expr.as_deref(), mode)
            }

            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field {
                            value,
                            type_annotation,
                            ..
                        } => {
                            if let Some(annotation) = type_annotation {
                                self.type_annotation(annotation, mode)?;
                            }
                            self.expr(value, mode)?;
                        }
                        ObjectEntry::Spread(inner) => self.expr(inner, mode)?,
                    }
                }
                Ok(())
            }

            Expr::Array(items, _) => self.exprs(items, mode),

            Expr::ListComprehension(comp, _) => {
                for clause in &comp.clauses {
                    self.destructure_pattern_binding(&clause.pattern, mode)?;
                    self.expr(&clause.iterable, mode)?;
                    if let Some(filter) = &clause.filter {
                        self.expr(filter, mode)?;
                    }
                }
                self.expr(&comp.element, mode)
            }

            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => self.variable_decl(decl, mode)?,
                        BlockItem::Assignment(assign) => self.assignment(assign, mode)?,
                        BlockItem::Statement(stmt) => self.statement(stmt, mode)?,
                        BlockItem::Expression(inner) => self.expr(inner, mode)?,
                    }
                }
                Ok(())
            }

            Expr::TypeAssertion {
                expr: inner,
                type_annotation,
                meta_param_overrides,
                span: _,
            } => {
                self.expr(inner, mode)?;
                self.type_annotation(type_annotation, mode)?;
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        self.expr(value, mode)?;
                    }
                }
                Ok(())
            }

            Expr::InstanceOf {
                expr: inner,
                type_annotation,
                span: _,
            } => {
                self.expr(inner, mode)?;
                self.type_annotation(type_annotation, mode)
            }

            // THE closure boundary: everything inside scans in
            // `ClosureInterior` mode — any occurrence of either name rejects.
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                captures,
                generated_origin: _,
                span: _,
            } => {
                for param in params {
                    self.function_parameter(param, ScanMode::ClosureInterior)?;
                }
                if let Some(ret) = return_type {
                    self.type_annotation(ret, ScanMode::ClosureInterior)?;
                }
                if let Some(clause) = captures {
                    for entry in &clause.entries {
                        self.check_binding_name(&entry.name, ScanMode::ClosureInterior)?;
                    }
                }
                self.statements(body, ScanMode::ClosureInterior)
            }

            Expr::Spread(inner, _) => self.expr(inner, mode),

            Expr::If(if_expr, _) => {
                self.expr(&if_expr.condition, mode)?;
                self.expr(&if_expr.then_branch, mode)?;
                self.opt_expr(if_expr.else_branch.as_deref(), mode)
            }

            Expr::While(while_expr, _) => {
                self.expr(&while_expr.condition, mode)?;
                self.expr(&while_expr.body, mode)
            }

            Expr::For(for_expr, _) => {
                self.match_pattern(&for_expr.pattern, mode)?;
                self.expr(&for_expr.iterable, mode)?;
                self.expr(&for_expr.body, mode)
            }

            Expr::Loop(loop_expr, _) => self.expr(&loop_expr.body, mode),

            Expr::Let(let_expr, _) => {
                self.match_pattern(&let_expr.pattern, mode)?;
                if let Some(annotation) = &let_expr.type_annotation {
                    self.type_annotation(annotation, mode)?;
                }
                self.opt_expr(let_expr.value.as_deref(), mode)?;
                self.expr(&let_expr.body, mode)
            }

            Expr::Assign(assign_expr, _) => {
                // `args[<int literal>] = expr` — the legal per-slot mutation
                // target. The target's own index legality is checked with the
                // same core as the read path.
                if mode == ScanMode::TemplateBody {
                    if let Expr::IndexAccess {
                        object,
                        index,
                        end_index,
                        span: _,
                    } = assign_expr.target.as_ref()
                    {
                        if self.is_args_identifier(object) {
                            self.args_index_access(index, end_index.as_deref())?;
                            return self.expr(&assign_expr.value, mode);
                        }
                    }
                }
                self.expr(&assign_expr.target, mode)?;
                self.expr(&assign_expr.value, mode)
            }

            Expr::Break(value, _) => self.opt_expr(value.as_deref(), mode),

            Expr::Return(value, _) => {
                // The parser's expression-position `return` twin: the same
                // authored `return args` spelling (see the fn docs).
                if mode == ScanMode::TemplateBody {
                    if let Some(inner) = value.as_deref() {
                        if self.is_args_identifier(inner) {
                            return Ok(());
                        }
                    }
                }
                self.opt_expr(value.as_deref(), mode)
            }

            Expr::MethodCall {
                receiver,
                method,
                args,
                named_args,
                optional: _,
                span: _,
            } => {
                self.check_reserved(method)?;
                self.expr(receiver, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::Match(match_expr, _) => {
                self.expr(&match_expr.scrutinee, mode)?;
                for arm in &match_expr.arms {
                    self.match_pattern(&arm.pattern, mode)?;
                    if let Some(guard) = &arm.guard {
                        self.expr(guard, mode)?;
                    }
                    self.expr(&arm.body, mode)?;
                }
                Ok(())
            }

            Expr::Range { start, end, .. } => {
                self.opt_expr(start.as_deref(), mode)?;
                self.opt_expr(end.as_deref(), mode)
            }

            Expr::TimeframeContext { expr: inner, .. } => self.expr(inner, mode),

            Expr::TryOperator(inner, _) => self.expr(inner, mode),

            Expr::UsingImpl { expr: inner, .. } => self.expr(inner, mode),

            Expr::SimulationCall { params, span: _, .. } => self.named_exprs(params, mode),

            Expr::WindowExpr(window, _) => self.window_expr(window, mode),

            Expr::FromQuery(query, _) => {
                self.check_binding_name(&query.variable, mode)?;
                self.expr(&query.source, mode)?;
                for clause in &query.clauses {
                    match clause {
                        QueryClause::Where(cond) => self.expr(cond, mode)?,
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.expr(&spec.key, mode)?;
                            }
                        }
                        QueryClause::GroupBy {
                            element,
                            key,
                            into_var,
                        } => {
                            self.expr(element, mode)?;
                            self.expr(key, mode)?;
                            if let Some(var) = into_var {
                                self.check_binding_name(var, mode)?;
                            }
                        }
                        QueryClause::Join {
                            variable,
                            source,
                            left_key,
                            right_key,
                            into_var,
                        } => {
                            self.check_binding_name(variable, mode)?;
                            self.expr(source, mode)?;
                            self.expr(left_key, mode)?;
                            self.expr(right_key, mode)?;
                            if let Some(var) = into_var {
                                self.check_binding_name(var, mode)?;
                            }
                        }
                        QueryClause::Let { variable, value } => {
                            self.check_binding_name(variable, mode)?;
                            self.expr(value, mode)?;
                        }
                    }
                }
                self.expr(&query.select, mode)
            }

            Expr::StructLiteral { fields, .. } => self.named_exprs(fields, mode),

            Expr::Await(inner, _) => self.expr(inner, mode),

            Expr::Join(join, _) => {
                for branch in &join.branches {
                    for annotation in &branch.annotations {
                        self.annotation_args(annotation, mode)?;
                    }
                    self.expr(&branch.expr, mode)?;
                }
                Ok(())
            }

            Expr::Annotated {
                annotation, target, ..
            } => {
                self.annotation_args(annotation, mode)?;
                self.expr(target, mode)
            }

            Expr::AsyncLet(async_let, _) => {
                self.check_binding_name(&async_let.name, mode)?;
                self.expr(&async_let.expr, mode)
            }

            Expr::AsyncScope(inner, _) => self.expr(inner, mode),

            Expr::Comptime(stmts, _) => self.statements(stmts, mode),

            Expr::ComptimeFor(comp_for, _) => {
                for witness in &comp_for.witnesses {
                    self.check_binding_name(witness, mode)?;
                }
                self.check_binding_name(&comp_for.variable, mode)?;
                self.expr(&comp_for.iterable, mode)?;
                self.statements(&comp_for.body, mode)
            }

            Expr::Reference { expr: inner, .. } => self.expr(inner, mode),

            Expr::TableRows(rows, _) => {
                for row in rows {
                    self.exprs(row, mode)?;
                }
                Ok(())
            }
        }
    }

    fn window_expr(&self, window: &WindowExpr, mode: ScanMode) -> Result<()> {
        match &window.function {
            WindowFunction::Lag { expr, default, .. }
            | WindowFunction::Lead { expr, default, .. } => {
                self.expr(expr, mode)?;
                self.opt_expr(default.as_deref(), mode)?;
            }
            WindowFunction::RowNumber
            | WindowFunction::Rank
            | WindowFunction::DenseRank
            | WindowFunction::Ntile(_) => {}
            WindowFunction::FirstValue(expr)
            | WindowFunction::LastValue(expr)
            | WindowFunction::NthValue(expr, _)
            | WindowFunction::Sum(expr)
            | WindowFunction::Avg(expr)
            | WindowFunction::Min(expr)
            | WindowFunction::Max(expr) => self.expr(expr, mode)?,
            WindowFunction::Count(expr) => self.opt_expr(expr.as_deref(), mode)?,
        }
        self.exprs(&window.over.partition_by, mode)?;
        if let Some(order_by) = &window.over.order_by {
            for (expr, _) in &order_by.columns {
                self.expr(expr, mode)?;
            }
        }
        // WindowFrame bounds carry no expressions (usize offsets only).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(src: &str) -> Vec<Statement> {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func.body),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn validate(src: &str) -> Result<()> {
        validate_pseudo_tuple_uses(&body_of(src), "args", "Args")
    }

    fn expect_reject(src: &str, needle: &str) {
        let err = validate(src).expect_err("fixture must be rejected");
        assert!(
            err.to_string().contains(needle),
            "expected rejection containing {needle:?}, got: {err}"
        );
    }

    // LEGAL: the full pseudo-tuple surface — constant-index read, constant-
    // index mutation, `.length`, and the `return args` mutation-return.
    #[test]
    fn full_legal_surface_validates() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] + 1
    let n = args.length
    if n > 1 {
        args[1] = 2
    }
    return args
}
"#,
        )
        .expect("the legal surface validates");
    }

    // LEGAL: a FINAL bare `args` expression statement is the implicit-return
    // tail spelling.
    #[test]
    fn final_bare_args_tail_is_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = 1
    args
}
"#,
        )
        .expect("final bare args tail is the implicit mutation-return");
    }

    // LEGAL: `return args` nested under control flow.
    #[test]
    fn return_args_inside_nested_block_is_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 0 {
        return args
    }
    return args
}
"#,
        )
        .expect("nested return args is legal");
    }

    // LEGAL: constant-index reads compose in ordinary expressions.
    #[test]
    fn constant_index_reads_in_expressions_are_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[0] + args[1]
    args[0] = x
    return args
}
"#,
        )
        .expect("constant-index reads are legal");
    }

    // NEGATIVE: non-constant index.
    #[test]
    fn non_constant_index_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let i = 0
    args[i] = 1
    return args
}
"#,
            "compile-time-constant index",
        );
    }

    // NEGATIVE: slicing (`args[0..1]` parses to `IndexAccess` with a slice
    // end).
    #[test]
    fn slicing_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[0..1]
    return args
}
"#,
            "cannot be sliced",
        );
    }

    // NEGATIVE: any property other than `length`.
    #[test]
    fn other_property_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args.first
    return args
}
"#,
            "has no property `first`",
        );
    }

    // NEGATIVE: bare `args` in a value position.
    #[test]
    fn bare_args_value_position_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: bare `args` as a NON-final expression statement is not the
    // tail spelling.
    #[test]
    fn bare_args_mid_body_expression_statement_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: whole-pack assignment (`args = e`).
    #[test]
    fn whole_pack_assignment_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args = 1
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: the pseudo-tuple does not cross closure boundaries.
    #[test]
    fn args_inside_closure_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x| x + args[0]
    return args
}
"#,
            "does not cross closure boundaries",
        );
    }

    // NEGATIVE: the type parameter does not cross closure boundaries either.
    #[test]
    fn type_param_inside_closure_annotation_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x: Args| x
    return args
}
"#,
            "does not cross closure boundaries",
        );
    }

    // NEGATIVE: the type parameter in a body-internal annotation.
    #[test]
    fn type_param_in_body_annotation_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x: Args = 0
    return args
}
"#,
            "cannot appear in a body-internal type annotation",
        );
    }

    // NEGATIVE: the reserved `__c3_` prefix anywhere in the body.
    #[test]
    fn reserved_prefix_identifier_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let __c3_tmp = 1
    return args
}
"#,
            "reserved prefix `__c3_`",
        );
    }

    // NEGATIVE: rebinding the pseudo-tuple parameter name.
    #[test]
    fn rebinding_args_param_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let args = 1
    return args
}
"#,
            "cannot be rebound",
        );
    }
}
