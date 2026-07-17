//! ADR-009 C2 #13 (slice 2) — the D6 conservative async-drop-context install
//! rejection (validation-battery row 10b; slice 3 assigns the `C0922` code).
//!
//! # What this rejects
//!
//! The battery's one GREENFIELD check (see [`super::battery`] row 10b). Shipped
//! lifecycle semantics prove sync `Drop`, the `drop_async` variant, and
//! `DropKind` context legality (battery rows 8/9/10a), but they do NOT prove a
//! drop-obligated value is safely handled when it is live ACROSS a suspension
//! point — that is the wave40 `AsyncDrop`/`MustSettle` protocol, which does not
//! exist yet. Rather than install a body whose soundness would depend on that
//! not-yet-existent protocol (C2-R6), C2 REJECTS it here, fail-closed.
//!
//! # Shape of the check (D6, supervisor-ruled)
//!
//! CONSERVATIVE, no liveness precision: a GENERATED body that contains BOTH
//!
//! - a drop-obligated binding — a local or parameter whose type carries a
//!   `Drop` impl (`drop_type_info` names it; the same drop-obligation query the
//!   emission drop-plan resolves through
//!   [`local_drop_kind`](BytecodeCompiler::local_drop_kind) /
//!   [`annotation_drop_kind`](BytecodeCompiler::annotation_drop_kind) /
//!   [`initializer_call_return_drop_type`](BytecodeCompiler::initializer_call_return_drop_type)),
//!   AND
//! - any suspension point (`await`, `async scope`, `async let`, `join`, or a
//!   `for await` loop),
//!
//! is REJECTED at install. It does NOT prove the value is provably live at the
//! suspension point — any drop-obligated binding plus any suspension point in
//! the same generated body rejects. This over-rejects and never installs
//! unsoundly; wave40 supplies the precision (and, when it lands, RELAXES these
//! rejections — nothing installed under this check can become retroactively
//! unsound). It is a NAMED installation rejection (slice 3 assigns the code),
//! never a soft-fail or a runtime fallback.
//!
//! # Placement and provenance gate
//!
//! The hook runs at the generated-body compile site (pass-2
//! `apply_comptime_extend` / `apply_comptime_extend_items`,
//! `functions_annotations.rs`), just before `compile_function`, so a rejection
//! rolls back atomically through the slice-1 install transaction — exactly the
//! path the pass-2 reject pin exercises. It is gated on AUTHENTICATED generated
//! provenance via the issuer-recognition capability
//! ([`GeneratedNodeIssuer::recognizes`](shape_ast::ast::GeneratedNodeIssuer),
//! the same authority `capture_plan/surface.rs` trusts), never a
//! function-name heuristic: only an origin this compiler instance issued is a
//! generated body we own. A SYNC generated body has no suspension point, and a
//! program with no `impl Drop` has no drop-obligated type, so the check is a
//! structural no-op for both — non-async / non-Drop programs stay
//! byte-identical.
//!
//! The AST walk is EXHAUSTIVE (no wildcard arm), mirroring
//! `transform::stamp_generated_closures`: a new `Expr`/`Statement` variant is a
//! compile failure here, so a future suspension- or binding-bearing node cannot
//! silently escape the fail-closed check.

use shape_ast::ast::expr_helpers::{BlockItem, ComprehensionClause, QueryClause};
use shape_ast::ast::expressions::EnumConstructorPayload;
use shape_ast::ast::statements::ForInit;
use shape_ast::ast::windows::{WindowExpr, WindowFunction, WindowSpec};
use shape_ast::ast::{Expr, FunctionDef, FunctionParameter, ObjectEntry, Statement, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::GeneratedOrigin;

impl BytecodeCompiler {
    /// The D6 conservative async-drop-context install guard for a generated
    /// body (see module docs). `Ok(())` means "not this check's concern"
    /// (sync body, no `Drop` type, or no drop-obligated binding across a
    /// suspension); `Err` is the named installation rejection.
    ///
    /// `origin` is the freshly-issued expansion provenance of the body being
    /// installed; the gate authenticates it against this compiler instance's
    /// issuer capability before policing anything (never a name heuristic).
    pub(in crate::compiler) fn reject_generated_drop_obligated_across_suspension(
        &self,
        func_def: &FunctionDef,
        origin: &GeneratedOrigin,
    ) -> Result<()> {
        // Provenance authentication (surface.rs pattern): only an origin THIS
        // compiler instance issued is a generated body we own. A foreign or
        // serialized origin is not recognized and is left alone.
        let node_origin = origin.to_node_origin(&self.generated_node_issuer, &func_def.name);
        if !self.generated_node_issuer.recognizes(&node_origin) {
            return Ok(());
        }

        // Structural no-ops: a sync body cannot hold a suspension point (an
        // `await` outside an async function is already a compile error), and a
        // program with no `impl Drop` has no drop-obligated type. Both keep
        // non-async / non-Drop programs byte-identical.
        if !func_def.is_async || self.drop_type_info.is_empty() {
            return Ok(());
        }

        let mut scan = AsyncDropContextScan::new(self);
        scan.note_params(&func_def.params);
        scan.walk_statements(&func_def.body);

        match (scan.saw_suspension, scan.drop_obligated_type) {
            (true, Some(type_name)) => Err(self.async_drop_context_rejection(&type_name)),
            _ => Ok(()),
        }
    }

    /// The named (code-free this slice; slice 3 assigns `C0922`) rejection. The
    /// message names BOTH facts and states the conservatism explicitly so the
    /// wave40 relaxation is documented at the point of refusal.
    fn async_drop_context_rejection(&self, type_name: &str) -> ShapeError {
        ShapeError::SemanticError {
            message: format!(
                "generated body holds a drop-obligated value of type `{type_name}` across a \
                 suspension point; installation is rejected pending the AsyncDrop protocol \
                 (wave40). This is a CONSERVATIVE, fail-closed rejection: any drop-obligated \
                 local or parameter plus any suspension point (await / async scope / async let \
                 / join / for-await) in the same generated body is refused, without liveness \
                 precision — the precise across-suspension analysis is wave40's."
            ),
            location: None,
        }
    }
}

/// Single-pass read-only scan collecting the two D6 facts over a generated
/// body: whether it contains a suspension point, and (the type name of) a
/// drop-obligated local/parameter if one exists. Drop-obligation is resolved
/// through the SAME queries the emission drop-plan uses, so the two agree.
struct AsyncDropContextScan<'c> {
    compiler: &'c BytecodeCompiler,
    saw_suspension: bool,
    drop_obligated_type: Option<String>,
}

impl<'c> AsyncDropContextScan<'c> {
    fn new(compiler: &'c BytecodeCompiler) -> Self {
        Self {
            compiler,
            saw_suspension: false,
            drop_obligated_type: None,
        }
    }

    /// Record a drop obligation if `type_name` carries a `Drop` impl. First hit
    /// wins (the type name is diagnostic-only; any drop-obligated binding
    /// triggers the same conservative rejection).
    fn note_type_name(&mut self, type_name: &str) {
        if self.drop_obligated_type.is_none() && self.compiler.drop_type_info.contains_key(type_name)
        {
            self.drop_obligated_type = Some(type_name.to_string());
        }
    }

    fn note_annotation(&mut self, annotation: &TypeAnnotation) {
        if let Some(type_name) = BytecodeCompiler::tracked_type_name_from_annotation(annotation) {
            self.note_type_name(&type_name);
        }
    }

    /// Note the drop obligation implied by one binding's declared type or its
    /// initializer — the drop-plan's own static resolution sources: an explicit
    /// annotation, a struct-literal initializer (the local's inferred type is
    /// the struct name), or a call initializer whose DECLARED return type is a
    /// `Drop` type. Deeper inference is out of scope by design (it matches what
    /// the drop-plan can statically resolve at this point).
    fn note_binding(&mut self, annotation: Option<&TypeAnnotation>, value: Option<&Expr>) {
        if self.drop_obligated_type.is_some() {
            return;
        }
        if let Some(annotation) = annotation {
            self.note_annotation(annotation);
        }
        if let Some(value) = value {
            if let Expr::StructLiteral { type_name, .. } = value {
                self.note_type_name(type_name.name());
            }
            if let Some((type_name, _drop_kind)) =
                self.compiler.initializer_call_return_drop_type(value)
            {
                self.note_type_name(&type_name);
            }
        }
    }

    fn note_params(&mut self, params: &[FunctionParameter]) {
        for param in params {
            if let Some(annotation) = param.type_annotation.as_ref() {
                self.note_annotation(annotation);
            }
            if let Some(default) = param.default_value.as_ref() {
                self.walk_expr(default);
            }
        }
    }

    fn walk_statements(&mut self, statements: &[Statement]) {
        for statement in statements {
            self.walk_statement(statement);
        }
    }

    fn walk_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Return(value, _) => {
                if let Some(value) = value {
                    self.walk_expr(value);
                }
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => {}
            Statement::VariableDecl(decl, _) => {
                self.note_binding(decl.type_annotation.as_ref(), decl.value.as_ref());
                if let Some(value) = decl.value.as_ref() {
                    self.walk_expr(value);
                }
            }
            Statement::Assignment(assign, _) => self.walk_expr(&assign.value),
            Statement::Expression(expr, _) => self.walk_expr(expr),
            Statement::For(for_loop, _) => {
                if for_loop.is_async {
                    self.saw_suspension = true;
                }
                match &for_loop.init {
                    ForInit::ForIn { pattern: _, iter } => self.walk_expr(iter),
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.walk_statement(init);
                        self.walk_expr(condition);
                        self.walk_expr(update);
                    }
                }
                self.walk_statements(&for_loop.body);
            }
            Statement::While(while_loop, _) => {
                self.walk_expr(&while_loop.condition);
                self.walk_statements(&while_loop.body);
            }
            Statement::If(if_stmt, _) => {
                self.walk_expr(&if_stmt.condition);
                self.walk_statements(&if_stmt.then_body);
                if let Some(else_body) = if_stmt.else_body.as_ref() {
                    self.walk_statements(else_body);
                }
            }
            Statement::Extend(extend, _) => {
                for method in &extend.methods {
                    for param in &method.params {
                        if let Some(default) = param.default_value.as_ref() {
                            self.walk_expr(default);
                        }
                    }
                    if let Some(when_clause) = method.when_clause.as_ref() {
                        self.walk_expr(when_clause);
                    }
                    self.walk_statements(&method.body);
                }
            }
            Statement::SetParamType { .. } | Statement::SetReturnType { .. } => {}
            Statement::SetParamTypeExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. }
            | Statement::ExtendItemsExpr { expression, .. } => self.walk_expr(expression),
            Statement::ReplaceBody { body, .. } => self.walk_statements(body),
        }
    }

    fn walk_exprs(&mut self, exprs: &[Expr]) {
        for expr in exprs {
            self.walk_expr(expr);
        }
    }

    fn walk_named(&mut self, named: &[(String, Expr)]) {
        for (_, expr) in named {
            self.walk_expr(expr);
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            // ── suspension points ───────────────────────────────────────────
            Expr::Await(inner, _) | Expr::AsyncScope(inner, _) => {
                self.saw_suspension = true;
                self.walk_expr(inner);
            }
            Expr::AsyncLet(async_let, _) => {
                self.saw_suspension = true;
                self.walk_expr(&async_let.expr);
            }
            Expr::Join(join_expr, _) => {
                self.saw_suspension = true;
                for branch in &join_expr.branches {
                    for annotation in &branch.annotations {
                        self.walk_exprs(&annotation.args);
                    }
                    self.walk_expr(&branch.expr);
                }
            }

            // ── binding carriers (drop-obligation sites) ────────────────────
            Expr::Let(let_expr, _) => {
                self.note_binding(
                    let_expr.type_annotation.as_ref(),
                    let_expr.value.as_deref(),
                );
                if let Some(value) = let_expr.value.as_deref() {
                    self.walk_expr(value);
                }
                self.walk_expr(&let_expr.body);
            }

            // ── generated closures: descend (conservative, whole-body) ──────
            Expr::FunctionExpr { params, body, .. } => {
                for param in params.iter() {
                    if let Some(default) = param.default_value.as_ref() {
                        self.walk_expr(default);
                    }
                }
                self.walk_statements(body);
            }

            // ── leaves ──────────────────────────────────────────────────────
            Expr::Literal(_, _)
            | Expr::Identifier(_, _)
            | Expr::DataRef(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::TypeSyntax(_, _)
            | Expr::Duration(_, _)
            | Expr::Continue(_)
            | Expr::Unit(_) => {}

            // ── single-child carriers ───────────────────────────────────────
            Expr::DataRelativeAccess { reference, .. } => self.walk_expr(reference),
            Expr::PropertyAccess { object, .. } => self.walk_expr(object),
            Expr::UnaryOp { operand, .. } => self.walk_expr(operand),
            Expr::Spread(inner, _) => self.walk_expr(inner),
            Expr::TryOperator(inner, _) => self.walk_expr(inner),
            Expr::UsingImpl { expr: inner, .. } => self.walk_expr(inner),
            Expr::TypeAssertion {
                expr: inner,
                meta_param_overrides,
                ..
            } => {
                self.walk_expr(inner);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        self.walk_expr(value);
                    }
                }
            }
            Expr::InstanceOf { expr: inner, .. } => self.walk_expr(inner),
            Expr::TimeframeContext { expr: inner, .. } => self.walk_expr(inner),
            Expr::Reference { expr: inner, .. } => self.walk_expr(inner),
            Expr::Annotated {
                annotation, target, ..
            } => {
                self.walk_exprs(&annotation.args);
                self.walk_expr(target);
            }
            Expr::Break(value, _) | Expr::Return(value, _) => {
                if let Some(value) = value {
                    self.walk_expr(value);
                }
            }

            // ── multi-child carriers ────────────────────────────────────────
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.walk_expr(object);
                self.walk_expr(index);
                if let Some(end_index) = end_index {
                    self.walk_expr(end_index);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::FunctionCall {
                const_args,
                args,
                named_args,
                ..
            }
            | Expr::QualifiedFunctionCall {
                const_args,
                args,
                named_args,
                ..
            } => {
                self.walk_exprs(const_args);
                self.walk_exprs(args);
                self.walk_named(named_args);
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.walk_expr(receiver);
                self.walk_exprs(args);
                self.walk_named(named_args);
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                EnumConstructorPayload::Unit => {}
                EnumConstructorPayload::Tuple(values) => self.walk_exprs(values),
                EnumConstructorPayload::Struct(fields) => self.walk_named(fields),
            },
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.walk_expr(condition);
                self.walk_expr(then_expr);
                if let Some(else_expr) = else_expr {
                    self.walk_expr(else_expr);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => self.walk_expr(value),
                        ObjectEntry::Spread(value) => self.walk_expr(value),
                    }
                }
            }
            Expr::Array(elements, _) => self.walk_exprs(elements),
            Expr::TableRows(rows, _) => {
                for row in rows {
                    self.walk_exprs(row);
                }
            }
            Expr::StructLiteral { fields, .. } => self.walk_named(fields),
            Expr::SimulationCall { params, .. } => self.walk_named(params),
            Expr::ListComprehension(comprehension, _) => {
                self.walk_expr(&comprehension.element);
                for ComprehensionClause {
                    pattern: _,
                    iterable,
                    filter,
                } in &comprehension.clauses
                {
                    self.walk_expr(iterable);
                    if let Some(filter) = filter {
                        self.walk_expr(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => {
                            self.note_binding(
                                decl.type_annotation.as_ref(),
                                decl.value.as_ref(),
                            );
                            if let Some(value) = decl.value.as_ref() {
                                self.walk_expr(value);
                            }
                        }
                        BlockItem::Assignment(assign) => self.walk_expr(&assign.value),
                        BlockItem::Statement(statement) => self.walk_statement(statement),
                        BlockItem::Expression(expr) => self.walk_expr(expr),
                    }
                }
            }
            Expr::If(if_expr, _) => {
                self.walk_expr(&if_expr.condition);
                self.walk_expr(&if_expr.then_branch);
                if let Some(else_branch) = if_expr.else_branch.as_ref() {
                    self.walk_expr(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                self.walk_expr(&while_expr.condition);
                self.walk_expr(&while_expr.body);
            }
            Expr::For(for_expr, _) => {
                if for_expr.is_async {
                    self.saw_suspension = true;
                }
                self.walk_expr(&for_expr.iterable);
                self.walk_expr(&for_expr.body);
            }
            Expr::Loop(loop_expr, _) => self.walk_expr(&loop_expr.body),
            Expr::Assign(assign_expr, _) => {
                self.walk_expr(&assign_expr.target);
                self.walk_expr(&assign_expr.value);
            }
            Expr::Match(match_expr, _) => {
                self.walk_expr(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = arm.guard.as_ref() {
                        self.walk_expr(guard);
                    }
                    self.walk_expr(&arm.body);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.walk_expr(start);
                }
                if let Some(end) = end {
                    self.walk_expr(end);
                }
            }
            Expr::Comptime(body, _) => self.walk_statements(body),
            Expr::ComptimeFor(comptime_for, _) => {
                self.walk_expr(&comptime_for.iterable);
                self.walk_statements(&comptime_for.body);
            }
            Expr::FromQuery(query, _) => {
                self.walk_expr(&query.source);
                for clause in &query.clauses {
                    match clause {
                        QueryClause::Where(condition) => self.walk_expr(condition),
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.walk_expr(&spec.key);
                            }
                        }
                        QueryClause::GroupBy { element, key, .. } => {
                            self.walk_expr(element);
                            self.walk_expr(key);
                        }
                        QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.walk_expr(source);
                            self.walk_expr(left_key);
                            self.walk_expr(right_key);
                        }
                        QueryClause::Let { value, .. } => self.walk_expr(value),
                    }
                }
                self.walk_expr(&query.select);
            }
            Expr::WindowExpr(window, _) => self.walk_window(window),
        }
    }

    fn walk_window(&mut self, window: &WindowExpr) {
        let WindowExpr { function, over } = window;
        match function {
            WindowFunction::Lag {
                expr,
                default,
                offset: _,
            }
            | WindowFunction::Lead {
                expr,
                default,
                offset: _,
            } => {
                self.walk_expr(expr);
                if let Some(default) = default {
                    self.walk_expr(default);
                }
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
            | WindowFunction::Max(expr) => self.walk_expr(expr),
            WindowFunction::Count(expr) => {
                if let Some(expr) = expr {
                    self.walk_expr(expr);
                }
            }
        }
        let WindowSpec {
            partition_by,
            order_by,
            frame: _,
        } = over;
        self.walk_exprs(partition_by);
        if let Some(order_by) = order_by {
            for (key, _direction) in &order_by.columns {
                self.walk_expr(key);
            }
        }
    }
}
