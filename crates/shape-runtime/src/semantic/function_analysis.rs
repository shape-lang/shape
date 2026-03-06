//! Function, variable, and assignment analysis
//!
//! This module handles analysis of function definitions, variable declarations,
//! and assignment statements.

use shape_ast::ast::{
    Assignment, DestructurePattern, FunctionDef, Spanned, Statement, VariableDecl,
};
use shape_ast::error::Result;

use super::type_annotation_to_type_with_aliases;
use super::types;

/// Implementation of function and variable analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Analyze a function definition
    pub(super) fn analyze_function(&mut self, function: &FunctionDef) -> Result<()> {
        // IMPORTANT: Register the function BEFORE analyzing its body
        // This allows recursive function calls to find themselves in the symbol table.

        // First, collect parameter types (resolve type aliases via symbol table)
        let mut param_types = Vec::new();
        for param in &function.params {
            // Warn if parameter has no type annotation (gradual typing)
            if param.type_annotation.is_none() {
                let param_names = param.get_identifiers().join(", ");
                self.add_warning(
                    param.span(),
                    format!(
                        "Parameter '{}' in function '{}' has implicit 'any' type. Consider adding a type annotation.",
                        param_names, function.name
                    ),
                );
            }
            let param_type = param
                .type_annotation
                .as_ref()
                .map(|ann| type_annotation_to_type_with_aliases(ann, Some(&self.symbol_table)))
                .unwrap_or(types::Type::Unknown);
            param_types.push(param_type.clone());
        }

        // Determine initial return type (will be refined after body analysis)
        let initial_return_type = function
            .return_type
            .as_ref()
            .map(|ann| type_annotation_to_type_with_aliases(ann, Some(&self.symbol_table)))
            .unwrap_or(types::Type::Unknown);

        // Register the function BEFORE analyzing body (for recursive calls).
        // The pre-registration pass in `analyze()` may have already inserted
        // this name (to enable mutual recursion).  If so, consume the
        // pre-registration token so that a *second* function with the same
        // name is correctly rejected as a duplicate.
        if self.pre_registered_functions.remove(&function.name) {
            // First occurrence — pre-registration already put it in the
            // symbol table, nothing more to do.
        } else {
            // Either no pre-registration happened (incremental mode) or this
            // is a genuine duplicate.  Let the symbol table reject it.
            self.symbol_table.define_function_with_defaults(
                &function.name,
                param_types.clone(),
                initial_return_type,
                function
                    .params
                    .iter()
                    .map(|p| p.default_value.is_some())
                    .collect(),
            )?;
        }

        // Create a new scope for function parameters
        self.symbol_table.push_scope();

        // Register parameters in the function scope
        for param in &function.params {
            let param_type = param
                .type_annotation
                .as_ref()
                .map(|ann| type_annotation_to_type_with_aliases(ann, Some(&self.symbol_table)))
                .unwrap_or(types::Type::Unknown);
            // All function parameters are mutable by default in Shape.
            // This allows patterns like `fn reset(s) { s = ""; s }`.
            // Reference params (&x) write through to the caller; non-ref params
            // are local copies that can be reassigned within the function body.
            let var_kind = shape_ast::ast::VarKind::Var;
            // Define all variables from the pattern
            for name in param.get_identifiers() {
                self.symbol_table
                    .define_variable(&name, param_type.clone(), var_kind, true)?;
            }
        }

        // Analyze the function body statements
        let mut inferred_return_type = types::Type::Unknown;

        for stmt in &function.body {
            match stmt {
                Statement::Return(expr_opt, _) => {
                    if let Some(expr) = expr_opt {
                        inferred_return_type = self.check_expr_type(expr)?;
                    } else {
                        inferred_return_type = types::Type::Unknown; // void return
                    }
                }
                Statement::VariableDecl(decl, _) => {
                    self.analyze_variable_decl(decl)?;
                }
                Statement::Assignment(assign, _) => {
                    self.analyze_assignment(assign)?;
                }
                Statement::Expression(expr, _) => {
                    self.check_expr_type(expr)?;
                }
                Statement::For(for_loop, _) => {
                    self.analyze_for_loop(for_loop)?;
                }
                Statement::While(while_loop, _) => {
                    self.analyze_while_loop(while_loop)?;
                }
                Statement::If(if_stmt, _) => {
                    self.analyze_if_statement(if_stmt)?;
                }
                Statement::Break(_) | Statement::Continue(_) => {
                    // Break and continue are valid control flow, no analysis needed
                }
                Statement::Extend(ext, _) => {
                    for method in &ext.methods {
                        for stmt in &method.body {
                            self.analyze_statement(stmt)?;
                        }
                    }
                }
                Statement::RemoveTarget(_) => {}
                Statement::SetParamType { .. }
                | Statement::SetReturnType { .. }
                | Statement::SetReturnExpr { .. } => {}
                Statement::ReplaceModuleExpr { expression, .. } => {
                    self.check_expr_type(expression)?;
                }
                Statement::ReplaceBodyExpr { expression, .. } => {
                    self.check_expr_type(expression)?;
                }
                Statement::ReplaceBody { body, .. } => {
                    for stmt in body {
                        self.analyze_statement(stmt)?;
                    }
                }
            }
        }

        // If a return type was specified, validate it matches the inferred type
        if let Some(declared_type) = &function.return_type {
            let expected_type =
                type_annotation_to_type_with_aliases(declared_type, Some(&self.symbol_table));
            if !inferred_return_type.can_coerce_to(&expected_type)
                && expected_type != types::Type::Unknown
                && inferred_return_type != types::Type::Unknown
            {
                return Err(self.error_at(
                    function.name_span,
                    format!(
                        "Function '{}' declared return type {} but inferred {}",
                        function.name, expected_type, inferred_return_type
                    ),
                ));
            }
            // Note: We've already registered the function with the declared type,
            // so we don't need to track inferred_return_type here.
            let _ = expected_type; // Silence unused warning
        }

        // Pop the parameter scope
        self.symbol_table.pop_scope();

        // Note: Function was already registered before body analysis (for recursive calls)
        // The initial registration used the declared return type or Unknown,
        // which is sufficient for recursive call type checking.

        Ok(())
    }

    /// Analyze a variable declaration
    pub(super) fn analyze_variable_decl(&mut self, var_decl: &VariableDecl) -> Result<()> {
        // Type check the initializer if present
        let var_type = if let Some(value) = &var_decl.value {
            let expr_type = match self.check_expr_type(value) {
                Ok(t) => t,
                Err(e) => {
                    // Register variable with Unknown type to prevent cascading errors
                    let _ = self.register_pattern_variables(
                        &var_decl.pattern,
                        types::Type::Unknown,
                        var_decl.kind,
                        true,
                    );
                    return Err(e);
                }
            };

            // If type annotation is provided, verify it matches
            if let Some(type_annotation) = &var_decl.type_annotation {
                let declared_type =
                    type_annotation_to_type_with_aliases(type_annotation, Some(&self.symbol_table));
                // Check compatibility between declared and inferred types
                if !expr_type.can_coerce_to(&declared_type)
                    && declared_type != types::Type::Unknown
                    && expr_type != types::Type::Unknown
                {
                    return Err(self.error_at(
                        value.span(),
                        format!(
                            "Type mismatch: declared {} but got {}",
                            declared_type, expr_type
                        ),
                    ));
                }
                // Use declared type if available
                declared_type
            } else {
                expr_type
            }
        } else if let Some(type_annotation) = &var_decl.type_annotation {
            // Variable with type annotation but no initializer
            type_annotation_to_type_with_aliases(type_annotation, Some(&self.symbol_table))
        } else {
            // Variable declared without type or initializer - error in strict mode
            // For now, allow it as Unknown type
            types::Type::Unknown
        };

        // Register the variable(s) in the symbol table
        let is_initialized = var_decl.value.is_some();
        self.register_pattern_variables(
            &var_decl.pattern,
            var_type,
            var_decl.kind,
            is_initialized,
        )?;

        Ok(())
    }

    /// Register variables from a pattern
    pub(super) fn register_pattern_variables(
        &mut self,
        pattern: &DestructurePattern,
        var_type: types::Type,
        kind: shape_ast::ast::VarKind,
        is_initialized: bool,
    ) -> Result<()> {
        match pattern {
            DestructurePattern::Identifier(name, _) => {
                self.symbol_table
                    .define_variable(name, var_type, kind, is_initialized)?;
            }
            DestructurePattern::Array(patterns) => {
                // For array destructuring, each element gets the same type for now
                for pattern in patterns {
                    self.register_pattern_variables(
                        pattern,
                        var_type.clone(),
                        kind,
                        is_initialized,
                    )?;
                }
            }
            DestructurePattern::Object(fields) => {
                // For object destructuring, each field gets the same type for now
                for field in fields {
                    self.register_pattern_variables(
                        &field.pattern,
                        var_type.clone(),
                        kind,
                        is_initialized,
                    )?;
                }
            }
            DestructurePattern::Rest(pattern) => {
                // Rest patterns get array type
                self.register_pattern_variables(pattern, var_type, kind, is_initialized)?;
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition bindings - each gets the specified component type
                // For now, we use the overall type since we don't have full type resolution
                for binding in bindings {
                    self.symbol_table.define_variable(
                        &binding.name,
                        var_type.clone(),
                        kind,
                        is_initialized,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Analyze an assignment
    pub(super) fn analyze_assignment(&mut self, assignment: &Assignment) -> Result<()> {
        // Check the expression type
        let expr_type = self.check_expr_type(&assignment.value)?;

        // For assignments with patterns, check that variables exist
        self.check_pattern_assignment(&assignment.pattern, expr_type)?;

        Ok(())
    }
}
