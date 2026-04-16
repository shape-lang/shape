//! Closure support for Shape functions

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use shape_ast::ast::{FunctionDef, VarKind};
use shape_value::{ValueWord, ValueWordExt};

/// A closure captures a function definition along with its environment
#[derive(Debug, Clone)]
pub struct Closure {
    /// The function definition
    pub function: Arc<FunctionDef>,
    /// Captured environment (variable bindings from enclosing scope)
    pub captured_env: CapturedEnvironment,
}

impl PartialEq for Closure {
    fn eq(&self, other: &Self) -> bool {
        // Closures are equal if they refer to the same function definition
        // and have the same captured environment
        Arc::ptr_eq(&self.function, &other.function) && self.captured_env == other.captured_env
    }
}

/// Captured environment for a closure
#[derive(Debug, Clone)]
pub struct CapturedEnvironment {
    /// Captured variable bindings
    pub bindings: HashMap<String, CapturedBinding>,
    /// Parent environment (for nested closures)
    pub parent: Option<Box<CapturedEnvironment>>,
}

/// A captured variable binding
#[derive(Debug, Clone)]
pub struct CapturedBinding {
    /// The captured value
    pub value: ValueWord,
    /// The kind of variable (let, var, const)
    pub kind: VarKind,
    /// Whether this binding is mutable (for 'var' declarations)
    pub is_mutable: bool,
}

impl PartialEq for CapturedBinding {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.is_mutable == other.is_mutable
            && self.value.vw_equals(&other.value)
    }
}

impl PartialEq for CapturedEnvironment {
    fn eq(&self, other: &Self) -> bool {
        self.bindings == other.bindings && self.parent == other.parent
    }
}

impl Default for CapturedEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl CapturedEnvironment {
    /// Create a new empty captured environment
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            parent: None,
        }
    }

    /// Create a new environment with a parent
    pub fn with_parent(parent: CapturedEnvironment) -> Self {
        Self {
            bindings: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    /// Capture a variable from the current scope
    pub fn capture(&mut self, name: String, value: ValueWord, kind: VarKind) {
        let is_mutable = matches!(kind, VarKind::Var);
        self.bindings.insert(
            name,
            CapturedBinding {
                value,
                kind,
                is_mutable,
            },
        );
    }

    /// Look up a captured variable
    pub fn lookup(&self, name: &str) -> Option<&CapturedBinding> {
        self.bindings
            .get(name)
            .or_else(|| self.parent.as_ref().and_then(|p| p.lookup(name)))
    }

    /// Look up a captured variable mutably
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut CapturedBinding> {
        if self.bindings.contains_key(name) {
            self.bindings.get_mut(name)
        } else if let Some(parent) = &mut self.parent {
            parent.lookup_mut(name)
        } else {
            None
        }
    }

    /// Get all captured variable names (including from parent scopes)
    pub fn all_captured_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.bindings.keys().cloned().collect();

        if let Some(parent) = &self.parent {
            for name in parent.all_captured_names() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }

        names
    }
}

/// Environment analyzer to detect which variables need to be captured
pub struct EnvironmentAnalyzer {
    /// Stack of scopes, each containing defined variables
    scope_stack: Vec<HashMap<String, bool>>, // bool indicates if variable is defined in this scope
    /// Variables that need to be captured
    captured_vars: HashMap<String, usize>, // usize is the scope level where var is defined
    /// Captured variables that are assigned to (mutated) inside the closure
    mutated_captures: HashSet<String>,
    /// The scope level at which the current function was entered.
    /// Variables defined at or above this level are local to the function
    /// and should NOT be captured. Only variables below this level (in the
    /// outer/enclosing scope) need capturing.
    function_scope_level: usize,
}

impl Default for EnvironmentAnalyzer {
    fn default() -> Self {
        Self {
            scope_stack: vec![HashMap::new()],
            captured_vars: HashMap::new(),
            mutated_captures: HashSet::new(),
            function_scope_level: 1,
        }
    }
}

impl EnvironmentAnalyzer {
    pub fn new() -> Self {
        Self {
            scope_stack: vec![HashMap::new()], // Start with root scope
            captured_vars: HashMap::new(),
            mutated_captures: HashSet::new(),
            function_scope_level: 1, // Default: function scope is level 1 (after outer scope at 0)
        }
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// Exit the current scope
    pub fn exit_scope(&mut self) {
        self.scope_stack.pop();
    }

    /// Record that a variable is defined in the current scope
    pub fn define_variable(&mut self, name: &str) {
        if let Some(current_scope) = self.scope_stack.last_mut() {
            current_scope.insert(name.to_string(), true);
        }
    }

    /// Check if a variable reference needs to be captured.
    ///
    /// Only variables defined OUTSIDE the current function (below `function_scope_level`)
    /// need capturing. Variables in the same function but in an outer block scope
    /// (e.g., defined before an if/while/for block) are just local variables.
    pub fn check_variable_reference(&mut self, name: &str) {
        // Search from current scope upwards
        for (level, scope) in self.scope_stack.iter().enumerate().rev() {
            if scope.contains_key(name) {
                // Variable is defined at this scope level.
                // Only capture if it's from outside the function boundary.
                if level < self.function_scope_level {
                    self.captured_vars.insert(name.to_string(), level);
                }
                return;
            }
        }
    }

    /// Mark a captured variable as mutated (assigned to inside the closure).
    pub fn mark_capture_mutated(&mut self, name: &str) {
        // Only mark it if it's actually a captured variable (from outer scope)
        for (level, scope) in self.scope_stack.iter().enumerate().rev() {
            if scope.contains_key(name) {
                if level < self.function_scope_level {
                    self.captured_vars.insert(name.to_string(), level);
                    self.mutated_captures.insert(name.to_string());
                }
                return;
            }
        }
    }

    /// Get the list of variables that need to be captured
    pub fn get_captured_vars(&self) -> Vec<String> {
        self.captured_vars.keys().cloned().collect()
    }

    /// Get the set of captured variables that are mutated inside the closure
    pub fn get_mutated_captures(&self) -> HashSet<String> {
        self.mutated_captures.clone()
    }

    /// Analyze a function to determine which variables it captures
    pub fn analyze_function(function: &FunctionDef, outer_scope_vars: &[String]) -> Vec<String> {
        let mut analyzer = Self::new();

        // Add outer scope variables (scope level 0)
        for var in outer_scope_vars {
            analyzer.define_variable(var);
        }

        // Enter function scope (scope level 1)
        analyzer.enter_scope();
        // Mark this as the function boundary — variables at this level or deeper
        // are local to the function and should NOT be captured
        analyzer.function_scope_level = analyzer.scope_stack.len() - 1;

        // Add function parameters to the scope
        for param in &function.params {
            for name in param.get_identifiers() {
                analyzer.define_variable(&name);
            }
        }

        // Analyze function body
        for stmt in &function.body {
            analyzer.analyze_statement(stmt);
        }

        analyzer.get_captured_vars()
    }

    /// Analyze a function to determine which variables it captures and which are mutated.
    /// Returns `(captured_vars, mutated_captures)`.
    pub fn analyze_function_with_mutability(
        function: &FunctionDef,
        outer_scope_vars: &[String],
    ) -> (Vec<String>, HashSet<String>) {
        let mut analyzer = Self::new();

        // Add outer scope variables (scope level 0)
        for var in outer_scope_vars {
            analyzer.define_variable(var);
        }

        // Enter function scope (scope level 1)
        analyzer.enter_scope();
        analyzer.function_scope_level = analyzer.scope_stack.len() - 1;

        // Add function parameters to the scope
        for param in &function.params {
            for name in param.get_identifiers() {
                analyzer.define_variable(&name);
            }
        }

        // Analyze function body
        for stmt in &function.body {
            analyzer.analyze_statement(stmt);
        }

        (
            analyzer.get_captured_vars(),
            analyzer.get_mutated_captures(),
        )
    }

    /// Analyze a statement for variable references and definitions
    fn analyze_statement(&mut self, stmt: &shape_ast::ast::Statement) {
        use shape_ast::ast::Statement;

        match stmt {
            Statement::Return(expr, _) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr);
                }
            }
            Statement::Expression(expr, _) => {
                self.analyze_expr(expr);
            }
            Statement::VariableDecl(decl, _) => {
                // First analyze the initializer (if any) before defining the variable
                if let Some(value) = &decl.value {
                    self.analyze_expr(value);
                }
                // Then define the variable(s)
                if let Some(name) = decl.pattern.as_identifier() {
                    self.define_variable(name);
                } else {
                    // Define all variables bound by the pattern
                    for name in decl.pattern.get_identifiers() {
                        self.define_variable(&name);
                    }
                }
            }
            Statement::Assignment(assign, _) => {
                self.analyze_expr(&assign.value);
                if let Some(name) = assign.pattern.as_identifier() {
                    // Assignment to a captured variable — mark it as mutated
                    self.mark_capture_mutated(name);
                    self.check_variable_reference(name);
                } else {
                    // Check all variables referenced by the pattern
                    for name in assign.pattern.get_identifiers() {
                        self.mark_capture_mutated(&name);
                        self.check_variable_reference(&name);
                    }
                }
            }
            Statement::If(if_stmt, _) => {
                self.analyze_expr(&if_stmt.condition);
                self.enter_scope();
                for stmt in &if_stmt.then_body {
                    self.analyze_statement(stmt);
                }
                self.exit_scope();

                if let Some(else_body) = &if_stmt.else_body {
                    self.enter_scope();
                    for stmt in else_body {
                        self.analyze_statement(stmt);
                    }
                    self.exit_scope();
                }
            }
            Statement::While(while_loop, _) => {
                self.analyze_expr(&while_loop.condition);
                self.enter_scope();
                for stmt in &while_loop.body {
                    self.analyze_statement(stmt);
                }
                self.exit_scope();
            }
            Statement::For(for_loop, _) => {
                self.enter_scope();

                // Analyze loop initialization
                match &for_loop.init {
                    shape_ast::ast::ForInit::ForIn { pattern, iter } => {
                        self.analyze_expr(iter);
                        // Define all variables from the pattern
                        for name in pattern.get_identifiers() {
                            self.define_variable(&name);
                        }
                    }
                    shape_ast::ast::ForInit::ForC {
                        init: _,
                        condition,
                        update,
                    } => {
                        // For C-style loops, we'd need to analyze init statement
                        // For now, just analyze condition and update
                        self.analyze_expr(condition);
                        self.analyze_expr(update);
                    }
                }

                for stmt in &for_loop.body {
                    self.analyze_statement(stmt);
                }

                self.exit_scope();
            }
            Statement::Break(_) | Statement::Continue(_) => {
                // No variables to analyze
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    self.enter_scope();
                    self.define_variable("self");
                    for param in &method.params {
                        for name in param.get_identifiers() {
                            self.define_variable(&name);
                        }
                    }
                    for stmt in &method.body {
                        self.analyze_statement(stmt);
                    }
                    self.exit_scope();
                }
            }
            Statement::RemoveTarget(_) => {}
            Statement::SetParamType { .. }
            | Statement::SetReturnType { .. }
            | Statement::SetReturnExpr { .. } => {}
            Statement::SetParamValue { expression, .. } => {
                self.analyze_expr(expression);
            }
            Statement::ReplaceModuleExpr { expression, .. } => {
                self.analyze_expr(expression);
            }
            Statement::ReplaceBodyExpr { expression, .. } => {
                self.analyze_expr(expression);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    self.analyze_statement(stmt);
                }
            }
        }
    }

    /// Analyze an expression for variable references
    fn analyze_expr(&mut self, expr: &shape_ast::ast::Expr) {
        use shape_ast::ast::Expr;

        match expr {
            Expr::Identifier(name, _) => {
                self.check_variable_reference(name);
            }
            Expr::Literal(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::PatternRef(..) => {
                // No variables to analyze
            }
            Expr::DataRelativeAccess {
                reference,
                index: _,
                ..
            } => {
                self.analyze_expr(reference);
                // Index is DataIndex, not an expression
            }
            Expr::BinaryOp { left, right, .. } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::FuzzyComparison { left, right, .. } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::UnaryOp { operand, .. } => {
                self.analyze_expr(operand);
            }
            Expr::FunctionCall { name, args, .. } => {
                // The function name might be a captured variable (e.g., a function
                // parameter holding a callable value like in `fn compose(f, g) { |x| f(g(x)) }`).
                self.check_variable_reference(name);
                for arg in args {
                    self.analyze_expr(arg);
                }
            }
            Expr::QualifiedFunctionCall {
                namespace,
                args,
                ..
            } => {
                self.check_variable_reference(namespace);
                for arg in args {
                    self.analyze_expr(arg);
                }
            }
            Expr::EnumConstructor { payload, .. } => {
                use shape_ast::ast::EnumConstructorPayload;
                match payload {
                    EnumConstructorPayload::Unit => {}
                    EnumConstructorPayload::Tuple(values) => {
                        for value in values {
                            self.analyze_expr(value);
                        }
                    }
                    EnumConstructorPayload::Struct(fields) => {
                        for (_, value) in fields {
                            self.analyze_expr(value);
                        }
                    }
                }
            }
            Expr::PropertyAccess { object, .. } => {
                self.analyze_expr(object);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.analyze_expr(condition);
                self.analyze_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.analyze_expr(else_e);
                }
            }
            Expr::Array(elements, _) => {
                for elem in elements {
                    self.analyze_expr(elem);
                }
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for elem in row {
                        self.analyze_expr(elem);
                    }
                }
            }
            Expr::ListComprehension(comp, _) => {
                // Analyze the element expression in a new scope
                self.enter_scope();

                // Process each clause
                for clause in &comp.clauses {
                    // The pattern creates new bindings
                    for name in clause.pattern.get_identifiers() {
                        self.define_variable(&name);
                    }

                    // Analyze the iterable
                    self.analyze_expr(&clause.iterable);

                    // Analyze the filter if present
                    if let Some(filter) = &clause.filter {
                        self.analyze_expr(filter);
                    }
                }

                // Analyze the element expression
                self.analyze_expr(&comp.element);

                self.exit_scope();
            }
            Expr::Object(entries, _) => {
                use shape_ast::ast::ObjectEntry;
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => self.analyze_expr(value),
                        ObjectEntry::Spread(spread_expr) => self.analyze_expr(spread_expr),
                    }
                }
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
                if let Some(end) = end_index {
                    self.analyze_expr(end);
                }
            }
            Expr::Block(block, _) => {
                self.enter_scope();
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                self.analyze_expr(value);
                            }
                            if let Some(name) = decl.pattern.as_identifier() {
                                self.define_variable(name);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            self.analyze_expr(&assign.value);
                            if let Some(name) = assign.pattern.as_identifier() {
                                self.mark_capture_mutated(name);
                                self.check_variable_reference(name);
                            } else {
                                for name in assign.pattern.get_identifiers() {
                                    self.mark_capture_mutated(&name);
                                    self.check_variable_reference(&name);
                                }
                            }
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            self.analyze_statement(stmt);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            self.analyze_expr(expr);
                        }
                    }
                }
                self.exit_scope();
            }
            Expr::TypeAssertion { expr, .. } => {
                self.analyze_expr(expr);
            }
            Expr::InstanceOf { expr, .. } => {
                self.analyze_expr(expr);
            }
            Expr::FunctionExpr {
                params,
                return_type: _,
                body,
                ..
            } => {
                // Enter nested function scope — save/restore function boundary
                let saved_function_scope_level = self.function_scope_level;
                self.enter_scope();
                self.function_scope_level = self.scope_stack.len() - 1;

                for param in params {
                    for name in param.get_identifiers() {
                        self.define_variable(&name);
                    }
                }

                for stmt in body {
                    self.analyze_statement(stmt);
                }

                self.exit_scope();
                self.function_scope_level = saved_function_scope_level;

                // Discard captures that belong to intermediate scopes (between the
                // outer function boundary and the nested function boundary).  These
                // are captures for the *nested* closure, not for the current one.
                self.captured_vars
                    .retain(|_, level| *level < saved_function_scope_level);
                self.mutated_captures
                    .retain(|name| self.captured_vars.contains_key(name));
            }
            Expr::Duration(..) => {
                // Duration literals have no variables to analyze
            }

            // Expression-based control flow
            Expr::If(if_expr, _) => {
                self.analyze_expr(&if_expr.condition);
                self.analyze_expr(&if_expr.then_branch);
                if let Some(else_branch) = &if_expr.else_branch {
                    self.analyze_expr(else_branch);
                }
            }

            Expr::While(while_expr, _) => {
                self.analyze_expr(&while_expr.condition);
                self.analyze_expr(&while_expr.body);
            }

            Expr::For(for_expr, _) => {
                self.enter_scope();
                // Pattern binding happens in scope
                self.analyze_pattern(&for_expr.pattern);
                self.analyze_expr(&for_expr.iterable);
                self.analyze_expr(&for_expr.body);
                self.exit_scope();
            }

            Expr::Loop(loop_expr, _) => {
                self.analyze_expr(&loop_expr.body);
            }

            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    self.analyze_expr(value);
                }
                self.enter_scope();
                self.analyze_pattern(&let_expr.pattern);
                self.analyze_expr(&let_expr.body);
                self.exit_scope();
            }

            Expr::Assign(assign, _) => {
                self.analyze_expr(&assign.value);
                self.analyze_expr(&assign.target);
            }

            Expr::Break(value, _) => {
                if let Some(val) = value {
                    self.analyze_expr(val);
                }
            }

            Expr::Continue(_) => {
                // No variables to analyze
            }

            Expr::Return(value, _) => {
                if let Some(val) = value {
                    self.analyze_expr(val);
                }
            }

            Expr::MethodCall { receiver, args, .. } => {
                self.analyze_expr(receiver);
                for arg in args {
                    self.analyze_expr(arg);
                }
            }

            Expr::Match(match_expr, _) => {
                self.analyze_expr(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    self.enter_scope();
                    self.analyze_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.analyze_expr(guard);
                    }
                    self.analyze_expr(&arm.body);
                    self.exit_scope();
                }
            }

            Expr::Unit(_) => {
                // Unit value has no variables to analyze
            }

            Expr::Spread(inner_expr, _) => {
                // Analyze the inner expression
                self.analyze_expr(inner_expr);
            }

            Expr::DateTime(..) => {
                // DateTime expressions have no variables to analyze
            }
            Expr::Range { start, end, .. } => {
                // Analyze both start and end expressions if present
                if let Some(s) = start {
                    self.analyze_expr(s);
                }
                if let Some(e) = end {
                    self.analyze_expr(e);
                }
            }

            Expr::TimeframeContext { expr, .. } => {
                // Analyze the inner expression
                self.analyze_expr(expr);
            }

            Expr::TryOperator(inner, _) => {
                // Analyze the inner expression
                self.analyze_expr(inner);
            }
            Expr::UsingImpl { expr, .. } => {
                self.analyze_expr(expr);
            }

            Expr::Await(inner, _) => {
                // Analyze the inner expression
                self.analyze_expr(inner);
            }

            Expr::SimulationCall { params, .. } => {
                // Simulation name is resolved at runtime by simulate()
                // Analyze all parameter value expressions
                for (_, value_expr) in params {
                    self.analyze_expr(value_expr);
                }
            }

            Expr::WindowExpr(_, _) => {
                // Window functions don't capture variables
            }

            Expr::FromQuery(from_query, _) => {
                // FromQuery should be desugared before closure analysis, but handle it anyway
                self.analyze_expr(&from_query.source);
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(pred) => {
                            self.analyze_expr(pred);
                        }
                        shape_ast::ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.analyze_expr(&spec.key);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            self.analyze_expr(element);
                            self.analyze_expr(key);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.analyze_expr(source);
                            self.analyze_expr(left_key);
                            self.analyze_expr(right_key);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            self.analyze_expr(value);
                        }
                    }
                }
                self.analyze_expr(&from_query.select);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value_expr) in fields {
                    self.analyze_expr(value_expr);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    self.analyze_expr(&branch.expr);
                }
            }
            Expr::Annotated { target, .. } => {
                self.analyze_expr(target);
            }
            Expr::AsyncLet(async_let, _) => {
                self.analyze_expr(&async_let.expr);
            }
            Expr::AsyncScope(inner, _) => {
                self.analyze_expr(inner);
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    self.analyze_statement(stmt);
                }
            }
            Expr::ComptimeFor(cf, _) => {
                self.analyze_expr(&cf.iterable);
                for stmt in &cf.body {
                    self.analyze_statement(stmt);
                }
            }
            Expr::Reference { expr: inner, .. } => {
                self.analyze_expr(inner);
            }
        }
    }

    /// Analyze a pattern for variable definitions
    fn analyze_pattern(&mut self, pattern: &shape_ast::ast::Pattern) {
        use shape_ast::ast::Pattern;

        match pattern {
            Pattern::Identifier(name) => {
                self.define_variable(name);
            }
            Pattern::Typed { name, .. } => {
                self.define_variable(name);
            }
            Pattern::Wildcard | Pattern::Literal(_) => {
                // No variables defined
            }
            Pattern::Array(patterns) => {
                for p in patterns {
                    self.analyze_pattern(p);
                }
            }
            Pattern::Object(fields) => {
                for (_, p) in fields {
                    self.analyze_pattern(p);
                }
            }
            Pattern::Constructor { fields, .. } => match fields {
                shape_ast::ast::PatternConstructorFields::Unit => {}
                shape_ast::ast::PatternConstructorFields::Tuple(patterns) => {
                    for p in patterns {
                        self.analyze_pattern(p);
                    }
                }
                shape_ast::ast::PatternConstructorFields::Struct(fields) => {
                    for (_, p) in fields {
                        self.analyze_pattern(p);
                    }
                }
            },
        }
    }
}
