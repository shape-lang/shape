//! Structural verifier for capture presentation spans.
//!
//! This index never resolves a name. It only proves that a compiler-issued
//! capture artifact's presentation span corresponds to the expected AST node
//! in the caller's source program.

use std::collections::HashSet;

use shape_ast::ast::{
    CaptureMode, DestructurePattern, Expr, Item, Pattern, Program, Span, Statement,
};
use shape_runtime::closure::callable_binding_span;
use shape_runtime::visitor::{Visitor, walk_program};

#[derive(Default)]
pub(super) struct AuthoredCaptureIndex {
    bindings: HashSet<(String, Span)>,
    declarations: HashSet<(String, CaptureMode, Span)>,
    uses: HashSet<(String, Span)>,
}

impl AuthoredCaptureIndex {
    pub(super) fn build(program: &Program) -> Self {
        let mut index = Self::default();
        walk_program(&mut index, program);
        index
    }

    pub(super) fn has_binding(&self, name: &str, span: Span) -> bool {
        self.bindings.contains(&(name.to_string(), span))
    }

    pub(super) fn has_declaration(&self, name: &str, mode: CaptureMode, span: Span) -> bool {
        self.declarations.contains(&(name.to_string(), mode, span))
    }

    pub(super) fn has_use(&self, name: &str, span: Span) -> bool {
        self.uses.contains(&(name.to_string(), span))
    }

    fn destructure_binding(&mut self, pattern: &DestructurePattern) {
        self.bindings.extend(pattern.get_bindings());
    }

    fn pattern_binding(&mut self, pattern: &Pattern) {
        self.bindings.extend(pattern.get_bindings());
    }
}

impl Visitor for AuthoredCaptureIndex {
    fn visit_item(&mut self, item: &Item) -> bool {
        match item {
            Item::VariableDecl(decl, _) => self.destructure_binding(&decl.pattern),
            Item::Function(function, _) => {
                for param in &function.params {
                    self.destructure_binding(&param.pattern);
                }
            }
            _ => {}
        }
        true
    }

    fn visit_stmt(&mut self, statement: &Statement) -> bool {
        match statement {
            Statement::VariableDecl(decl, _) => self.destructure_binding(&decl.pattern),
            Statement::Assignment(assignment, _) => {
                self.uses.extend(assignment.pattern.get_bindings());
            }
            Statement::For(for_loop, _) => match &for_loop.init {
                shape_ast::ast::ForInit::ForIn { pattern, .. } => {
                    self.destructure_binding(pattern);
                }
                shape_ast::ast::ForInit::ForC { .. } => {}
            },
            Statement::Extend(extension, _) => {
                for method in &extension.methods {
                    for param in &method.params {
                        self.destructure_binding(&param.pattern);
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn visit_expr(&mut self, expr: &Expr) -> bool {
        match expr {
            Expr::Identifier(name, span) => {
                self.uses.insert((name.clone(), *span));
            }
            Expr::FunctionCall { name, span, .. } => {
                if let Some(span) = callable_binding_span(name, *span) {
                    self.uses.insert((name.clone(), span));
                }
            }
            Expr::QualifiedFunctionCall {
                namespace, span, ..
            } => {
                if let Some(span) = callable_binding_span(namespace, *span) {
                    self.uses.insert((namespace.clone(), span));
                }
            }
            Expr::FunctionExpr {
                params, captures, ..
            } => {
                for param in params {
                    self.destructure_binding(&param.pattern);
                }
                if let Some(clause) = captures {
                    for entry in &clause.entries {
                        self.declarations
                            .insert((entry.name.clone(), entry.mode, entry.span));
                        // A nested explicit capture entry is simultaneously
                        // the inner occurrence's declaration and the outer
                        // closure's structural use that forwards the carrier.
                        // The canonical analyzer records that exact span for
                        // the outer pack; preserve both AST roles here.
                        self.uses.insert((entry.name.clone(), entry.span));
                    }
                }
            }
            Expr::For(for_expr, _) => self.pattern_binding(&for_expr.pattern),
            Expr::Let(let_expr, _) => self.pattern_binding(&let_expr.pattern),
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    self.pattern_binding(&arm.pattern);
                }
            }
            Expr::ListComprehension(comprehension, _) => {
                for clause in &comprehension.clauses {
                    self.destructure_binding(&clause.pattern);
                }
            }
            _ => {}
        }
        true
    }
}
