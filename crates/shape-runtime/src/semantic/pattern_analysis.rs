//! Pattern analysis and query validation
//!
//! This module handles analysis of pattern definitions and query constructs.

use shape_ast::ast::{DestructurePattern, Query, Span, Spanned};
use shape_ast::error::Result;

use super::types;

/// Implementation of pattern analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Check pattern assignment (variables must exist)
    pub(super) fn check_pattern_assignment(
        &mut self,
        pattern: &DestructurePattern,
        expr_type: types::Type,
    ) -> Result<()> {
        match pattern {
            DestructurePattern::Identifier(name, _) => {
                if self.symbol_table.lookup_variable(name).is_some() {
                    // Variable exists, update it
                    self.symbol_table.update_variable(name, expr_type)?;
                } else {
                    // Variable doesn't exist, create it as 'var' for backward compatibility
                    self.symbol_table.define_variable(
                        name,
                        expr_type,
                        shape_ast::ast::VarKind::Var,
                        true,
                    )?;
                }
            }
            DestructurePattern::Array(patterns) => {
                // For array destructuring in assignment
                for pattern in patterns {
                    self.check_pattern_assignment(pattern, expr_type.clone())?;
                }
            }
            DestructurePattern::Object(fields) => {
                // For object destructuring in assignment
                for field in fields {
                    self.check_pattern_assignment(&field.pattern, expr_type.clone())?;
                }
            }
            DestructurePattern::Rest(pattern) => {
                self.check_pattern_assignment(pattern, expr_type)?;
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition assigns component types from an intersection
                for binding in bindings {
                    if self.symbol_table.lookup_variable(&binding.name).is_some() {
                        self.symbol_table
                            .update_variable(&binding.name, expr_type.clone())?;
                    } else {
                        self.symbol_table.define_variable(
                            &binding.name,
                            expr_type.clone(),
                            shape_ast::ast::VarKind::Var,
                            true,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyze a query
    pub(super) fn analyze_query(&mut self, query: &Query, span: Span) -> Result<()> {
        match query {
            Query::Backtest(_) => {
                // Backtest validation would check strategy exists
            }
            Query::Alert(alert) => {
                // Alert condition must be boolean
                let cond_type = self.check_expr_type(&alert.condition)?;
                if cond_type != types::Type::Bool {
                    return Err(self.error_at(
                        alert.condition.span(),
                        format!("Alert condition must be boolean, got {:?}", cond_type),
                    ));
                }
            }
            Query::With(with_query) => {
                // Analyze CTEs
                for cte in &with_query.ctes {
                    self.analyze_query(&cte.query, span)?;
                }
                // Analyze main query
                self.analyze_query(&with_query.query, span)?;
            }
        }

        Ok(())
    }

    /// Get list of all available patterns (user-defined + built-in)
    pub fn list_available_patterns(&self) -> Vec<String> {
        let mut patterns = self.pattern_library.pattern_names();
        patterns.sort();
        patterns.dedup();
        patterns
    }
}
