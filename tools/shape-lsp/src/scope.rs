//! Scope-aware symbol resolution for Shape
//!
//! Builds a scope tree from the AST for accurate find-references and rename
//! operations that respect variable shadowing and lexical scoping.

use shape_ast::ast::{BlockItem, Expr, Item, Pattern, Program, Span, Spanned, Statement};

/// A binding (variable/function definition) within a scope.
#[derive(Debug, Clone)]
pub struct Binding {
    /// The identifier name
    pub name: String,
    /// Byte span of the definition site
    pub def_span: (usize, usize),
    /// Byte spans of all reference sites (excludes the definition itself)
    pub references: Vec<(usize, usize)>,
}

/// A single lexical scope.
#[derive(Debug)]
pub struct Scope {
    /// Byte range this scope covers
    pub range: (usize, usize),
    /// Parent scope index (None for the module scope)
    pub parent: Option<usize>,
    /// Bindings introduced in this scope
    pub bindings: Vec<Binding>,
    /// Child scope indices
    pub children: Vec<usize>,
}

/// A tree of lexical scopes built from a parsed program.
#[derive(Debug)]
pub struct ScopeTree {
    pub scopes: Vec<Scope>,
}

impl ScopeTree {
    /// Build a scope tree from a parsed program.
    pub fn build(program: &Program, source: &str) -> Self {
        let mut tree = ScopeTree { scopes: Vec::new() };

        // Create the module (root) scope covering the entire source
        let root = tree.push_scope(0, source.len(), None);

        // First pass: collect all definitions
        for item in &program.items {
            tree.collect_item_definitions(item, root);
        }

        // Second pass: collect all identifier references
        for item in &program.items {
            tree.collect_item_references(item, root);
        }

        tree
    }

    /// Find all references to the binding at the given byte offset.
    ///
    /// Returns the definition span + all reference spans, or None if no binding
    /// is found at the offset.
    pub fn references_of(&self, offset: usize) -> Option<Vec<(usize, usize)>> {
        // Find which binding the offset falls within
        let binding = self.find_binding_at(offset)?;

        let mut result = vec![binding.def_span];
        result.extend_from_slice(&binding.references);
        Some(result)
    }

    /// Find the definition span of the binding at the given byte offset.
    pub fn definition_of(&self, offset: usize) -> Option<(usize, usize)> {
        let binding = self.find_binding_at(offset)?;
        Some(binding.def_span)
    }

    /// Return the binding (definition + references) visible at the given
    /// offset. Used by hover to resolve shadowed variables correctly.
    pub fn binding_at(&self, offset: usize) -> Option<&Binding> {
        self.find_binding_at(offset)
    }

    // --- Internal helpers ---

    fn push_scope(&mut self, start: usize, end: usize, parent: Option<usize>) -> usize {
        let idx = self.scopes.len();
        self.scopes.push(Scope {
            range: (start, end),
            parent,
            bindings: Vec::new(),
            children: Vec::new(),
        });
        if let Some(parent_idx) = parent {
            self.scopes[parent_idx].children.push(idx);
        }
        idx
    }

    fn add_binding(&mut self, scope_idx: usize, name: String, def_span: (usize, usize)) {
        self.scopes[scope_idx].bindings.push(Binding {
            name,
            def_span,
            references: Vec::new(),
        });
    }

    /// Find a binding at the given byte offset.
    /// Checks definitions and references in all scopes.
    fn find_binding_at(&self, offset: usize) -> Option<&Binding> {
        for scope in &self.scopes {
            for binding in &scope.bindings {
                // Check if offset is on the definition
                if offset >= binding.def_span.0 && offset < binding.def_span.1 {
                    return Some(binding);
                }
                // Check if offset is on any reference
                for &(start, end) in &binding.references {
                    if offset >= start && offset < end {
                        return Some(binding);
                    }
                }
            }
        }
        None
    }

    /// Resolve a name in the given scope, walking up to parent scopes.
    fn resolve_name(&self, name: &str, scope_idx: usize) -> Option<(usize, usize)> {
        let scope = &self.scopes[scope_idx];
        // Search current scope bindings (last binding with this name wins — shadowing)
        for binding in scope.bindings.iter().rev() {
            if binding.name == name {
                return Some((
                    scope_idx,
                    scope.bindings.iter().rposition(|b| b.name == name).unwrap(),
                ));
            }
        }
        // Walk up to parent
        if let Some(parent) = scope.parent {
            self.resolve_name(name, parent)
        } else {
            None
        }
    }

    /// Add a reference to a binding (if it exists in scope).
    fn add_reference(&mut self, name: &str, ref_span: (usize, usize), scope_idx: usize) {
        if let Some((scope_id, binding_id)) = self.resolve_name(name, scope_idx) {
            self.scopes[scope_id].bindings[binding_id]
                .references
                .push(ref_span);
        }
    }

    /// Find the innermost scope containing the given byte offset.
    fn scope_at(&self, offset: usize) -> usize {
        let mut best = 0; // root scope
        for (idx, scope) in self.scopes.iter().enumerate() {
            if offset >= scope.range.0
                && offset < scope.range.1
                && (scope.range.1 - scope.range.0)
                    < (self.scopes[best].range.1 - self.scopes[best].range.0)
            {
                best = idx;
            }
        }
        best
    }

    // --- Definition collection ---

    fn collect_item_definitions(&mut self, item: &Item, scope_idx: usize) {
        match item {
            Item::Function(func, _span) => {
                // The function name is defined in the outer scope
                let name_span = func.name_span;
                if !name_span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        func.name.clone(),
                        (name_span.start, name_span.end),
                    );
                }

                // Create a new scope for the function body
                let func_span = _span;
                let func_scope = self.push_scope(func_span.start, func_span.end, Some(scope_idx));

                // Parameters are bindings in the function scope
                for param in &func.params {
                    if let Some(name) = param.simple_name() {
                        let ps = param.span();
                        if !ps.is_dummy() {
                            self.add_binding(func_scope, name.to_string(), (ps.start, ps.end));
                        }
                    }
                }

                // Walk the body
                for stmt in &func.body {
                    self.collect_stmt_definitions(stmt, func_scope);
                }
            }
            Item::VariableDecl(decl, span) => {
                self.collect_var_decl_def(decl, span, scope_idx);
            }
            Item::Statement(stmt, _) => {
                self.collect_stmt_definitions(stmt, scope_idx);
            }
            // Types, enums, traits, etc. define names in the module scope
            Item::StructType(s, span) => {
                if !span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        s.name.clone(),
                        (span.start, span.start + s.name.len()),
                    );
                }
            }
            Item::Enum(e, span) => {
                if !span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        e.name.clone(),
                        (span.start, span.start + e.name.len()),
                    );
                }
            }
            Item::Trait(t, span) => {
                if !span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        t.name.clone(),
                        (span.start, span.start + t.name.len()),
                    );
                }
            }
            Item::TypeAlias(ta, span) => {
                if !span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        ta.name.clone(),
                        (span.start, span.start + ta.name.len()),
                    );
                }
            }
            Item::ForeignFunction(foreign_fn, _span) => {
                let name_span = foreign_fn.name_span;
                if !name_span.is_dummy() {
                    self.add_binding(
                        scope_idx,
                        foreign_fn.name.clone(),
                        (name_span.start, name_span.end),
                    );
                }
            }
            _ => {}
        }
    }

    fn collect_stmt_definitions(&mut self, stmt: &Statement, scope_idx: usize) {
        match stmt {
            Statement::VariableDecl(decl, span) => {
                self.collect_var_decl_def(decl, span, scope_idx);
            }
            Statement::Expression(expr, _) => {
                self.collect_expr_definitions(expr, scope_idx);
            }
            _ => {}
        }
    }

    fn collect_var_decl_def(
        &mut self,
        decl: &shape_ast::ast::VariableDecl,
        _span: &Span,
        scope_idx: usize,
    ) {
        for (name, span) in crate::symbols::get_pattern_names(&decl.pattern) {
            if !span.is_dummy() {
                self.add_binding(scope_idx, name, (span.start, span.end));
            }
        }

        // Walk value expression for nested definitions
        if let Some(value) = &decl.value {
            self.collect_expr_definitions(value, scope_idx);
        }
    }

    fn collect_expr_definitions(&mut self, expr: &Expr, scope_idx: usize) {
        match expr {
            Expr::Block(block, span) => {
                let block_scope = self.push_scope(span.start, span.end, Some(scope_idx));
                for item in &block.items {
                    match item {
                        BlockItem::Statement(stmt) => {
                            self.collect_stmt_definitions(stmt, block_scope);
                        }
                        BlockItem::Expression(e) => {
                            self.collect_expr_definitions(e, block_scope);
                        }
                        BlockItem::VariableDecl(vd) => {
                            self.collect_stmt_definitions(
                                &shape_ast::ast::Statement::VariableDecl(
                                    vd.clone(),
                                    shape_ast::ast::Span::DUMMY,
                                ),
                                block_scope,
                            );
                        }
                        BlockItem::Assignment(_) => {
                            // Assignments don't introduce new definitions
                        }
                    }
                }
            }
            Expr::For(for_expr, span) => {
                let for_scope = self.push_scope(span.start, span.end, Some(scope_idx));
                // Loop variable from pattern
                if let Pattern::Identifier(name) = &for_expr.pattern {
                    // Use the span start to approximate the variable position
                    // (Pattern doesn't carry its own span, use the for-expr span start)
                    let name_start = span.start;
                    let name_end = name_start + name.len();
                    self.add_binding(for_scope, name.clone(), (name_start, name_end));
                }
                self.collect_expr_definitions(&for_expr.body, for_scope);
            }
            Expr::If(if_expr, _) => {
                self.collect_expr_definitions(&if_expr.then_branch, scope_idx);
                if let Some(else_branch) = &if_expr.else_branch {
                    self.collect_expr_definitions(else_branch, scope_idx);
                }
            }
            Expr::FunctionExpr {
                body, params, span, ..
            } => {
                let closure_scope = self.push_scope(span.start, span.end, Some(scope_idx));
                for param in params {
                    if let Some(name) = param.simple_name() {
                        let ps = param.span();
                        if !ps.is_dummy() {
                            self.add_binding(closure_scope, name.to_string(), (ps.start, ps.end));
                        }
                    }
                }
                for stmt in body {
                    self.collect_stmt_definitions(stmt, closure_scope);
                }
            }
            Expr::Match(match_expr, _span) => {
                for arm in &match_expr.arms {
                    // Each arm creates a scope for pattern bindings
                    let arm_span = (*arm.body).span();
                    if !arm_span.is_dummy() {
                        let arm_scope =
                            self.push_scope(arm_span.start, arm_span.end, Some(scope_idx));
                        self.collect_pattern_bindings(
                            &arm.pattern,
                            arm.pattern_span.as_ref(),
                            arm_scope,
                        );
                        self.collect_expr_definitions(&arm.body, arm_scope);
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_pattern_bindings(
        &mut self,
        pattern: &shape_ast::ast::Pattern,
        pattern_span: Option<&Span>,
        scope_idx: usize,
    ) {
        match pattern {
            shape_ast::ast::Pattern::Identifier(name) => {
                if let Some(span) = pattern_span {
                    if !span.is_dummy() {
                        let start = span.start;
                        let end = start.saturating_add(name.len());
                        self.add_binding(scope_idx, name.clone(), (start, end));
                    }
                }
            }
            shape_ast::ast::Pattern::Typed {
                name,
                type_annotation: _,
            } => {
                if let Some(span) = pattern_span {
                    if !span.is_dummy() {
                        let start = span.start;
                        let end = start.saturating_add(name.len());
                        self.add_binding(scope_idx, name.clone(), (start, end));
                    }
                }
            }
            shape_ast::ast::Pattern::Constructor { .. } => {
                // Recurse into constructor pattern fields
            }
            _ => {}
        }
    }

    // --- Reference collection ---

    fn collect_item_references(&mut self, item: &Item, scope_idx: usize) {
        match item {
            Item::Function(func, span) => {
                let func_scope = self.find_child_scope(scope_idx, span);
                for stmt in &func.body {
                    self.collect_stmt_references(stmt, func_scope);
                }
            }
            Item::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_expr_references(value, scope_idx);
                }
            }
            Item::Statement(stmt, _) => {
                self.collect_stmt_references(stmt, scope_idx);
            }
            Item::Expression(expr, _) => {
                self.collect_expr_references(expr, scope_idx);
            }
            _ => {}
        }
    }

    fn collect_stmt_references(&mut self, stmt: &Statement, scope_idx: usize) {
        match stmt {
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_expr_references(value, scope_idx);
                }
            }
            Statement::Expression(expr, _) => {
                self.collect_expr_references(expr, scope_idx);
            }
            Statement::Return(Some(expr), _) => {
                self.collect_expr_references(expr, scope_idx);
            }
            _ => {}
        }
    }

    fn collect_expr_references(&mut self, expr: &Expr, scope_idx: usize) {
        match expr {
            Expr::Identifier(name, span) => {
                if !span.is_dummy() {
                    self.add_reference(name, (span.start, span.end), scope_idx);
                }
            }
            Expr::Block(block, span) => {
                let block_scope = self.find_child_scope(scope_idx, span);
                for item in &block.items {
                    match item {
                        BlockItem::Statement(stmt) => {
                            self.collect_stmt_references(stmt, block_scope);
                        }
                        BlockItem::Expression(e) => {
                            self.collect_expr_references(e, block_scope);
                        }
                        BlockItem::VariableDecl(vd) => {
                            self.collect_stmt_references(
                                &shape_ast::ast::Statement::VariableDecl(
                                    vd.clone(),
                                    shape_ast::ast::Span::DUMMY,
                                ),
                                block_scope,
                            );
                        }
                        BlockItem::Assignment(assign) => {
                            self.collect_expr_references(&assign.value, block_scope);
                        }
                    }
                }
            }
            Expr::For(for_expr, span) => {
                let for_scope = self.find_child_scope(scope_idx, span);
                self.collect_expr_references(&for_expr.iterable, scope_idx);
                self.collect_expr_references(&for_expr.body, for_scope);
            }
            Expr::If(if_expr, _) => {
                self.collect_expr_references(&if_expr.condition, scope_idx);
                self.collect_expr_references(&if_expr.then_branch, scope_idx);
                if let Some(else_branch) = &if_expr.else_branch {
                    self.collect_expr_references(else_branch, scope_idx);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.collect_expr_references(left, scope_idx);
                self.collect_expr_references(right, scope_idx);
            }
            Expr::UnaryOp { operand, .. } => {
                self.collect_expr_references(operand, scope_idx);
            }
            Expr::FunctionCall {
                name, args, span, ..
            } => {
                // The function name is a reference
                if !span.is_dummy() {
                    // Find the start of the function name in the source
                    let name_start = span.start;
                    let name_end = name_start + name.len();
                    self.add_reference(name, (name_start, name_end), scope_idx);
                }
                for arg in args {
                    self.collect_expr_references(arg, scope_idx);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.collect_expr_references(receiver, scope_idx);
                for arg in args {
                    self.collect_expr_references(arg, scope_idx);
                }
            }
            Expr::PropertyAccess { object, .. } => {
                self.collect_expr_references(object, scope_idx);
            }
            Expr::IndexAccess { object, index, .. } => {
                self.collect_expr_references(object, scope_idx);
                self.collect_expr_references(index, scope_idx);
            }
            Expr::Array(elements, _) => {
                for el in elements {
                    self.collect_expr_references(el, scope_idx);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    if let shape_ast::ast::ObjectEntry::Field { value, .. } = entry {
                        self.collect_expr_references(value, scope_idx);
                    }
                }
            }
            Expr::Assign(assign, _) => {
                self.collect_expr_references(&assign.target, scope_idx);
                self.collect_expr_references(&assign.value, scope_idx);
            }
            Expr::Return(Some(inner), _) => {
                self.collect_expr_references(inner, scope_idx);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.collect_expr_references(condition, scope_idx);
                self.collect_expr_references(then_expr, scope_idx);
                if let Some(else_e) = else_expr {
                    self.collect_expr_references(else_e, scope_idx);
                }
            }
            Expr::Match(match_expr, _span) => {
                self.collect_expr_references(&match_expr.scrutinee, scope_idx);
                for arm in &match_expr.arms {
                    let arm_span = (*arm.body).span();
                    let arm_scope = if !arm_span.is_dummy() {
                        self.find_child_scope(scope_idx, &arm_span)
                    } else {
                        scope_idx
                    };
                    self.collect_expr_references(&arm.body, arm_scope);
                }
            }
            Expr::Literal(_, _) => {}
            Expr::FunctionExpr { body, span, .. } => {
                let closure_scope = self.find_child_scope(scope_idx, span);
                for stmt in body {
                    self.collect_stmt_references(stmt, closure_scope);
                }
            }
            Expr::Await(inner, _) => {
                self.collect_expr_references(inner, scope_idx);
            }
            Expr::TryOperator(inner, _) => {
                self.collect_expr_references(inner, scope_idx);
            }
            Expr::TypeAssertion { expr: inner, .. } => {
                self.collect_expr_references(inner, scope_idx);
            }
            Expr::Spread(inner, _) => {
                self.collect_expr_references(inner, scope_idx);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.collect_expr_references(value, scope_idx);
                }
            }
            _ => {
                // For other expression types, we don't recurse deeper
            }
        }
    }

    /// Find a child scope of `parent_idx` that matches the given span.
    /// Falls back to `parent_idx` if no matching child is found.
    fn find_child_scope(&self, parent_idx: usize, span: &Span) -> usize {
        for &child in &self.scopes[parent_idx].children {
            let cs = &self.scopes[child];
            if cs.range.0 == span.start && cs.range.1 == span.end {
                return child;
            }
        }
        // Fallback: find the innermost scope containing this span
        self.scope_at(span.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_scope_tree_basic_variable() {
        let code = "let x = 42\nlet y = x + 1";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // Root scope should exist
        assert!(!tree.scopes.is_empty());

        // x should have 1 reference (the use in `y = x + 1`)
        let x_binding = tree.scopes[0].bindings.iter().find(|b| b.name == "x");
        assert!(x_binding.is_some(), "Should find binding for 'x'");
        let x = x_binding.unwrap();
        assert!(
            !x.references.is_empty(),
            "x should have at least one reference, got: {:?}",
            x
        );
    }

    #[test]
    fn test_scope_tree_function_scope() {
        let code = "let x = 1\nfn foo(a) {\n  let b = a + x\n  return b\n}";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // Should have at least 2 scopes: root + function body
        assert!(
            tree.scopes.len() >= 2,
            "Should have root + function scope, got {}",
            tree.scopes.len()
        );

        // `a` should be bound in the function scope
        let func_scope = &tree.scopes[1];
        let a_binding = func_scope.bindings.iter().find(|b| b.name == "a");
        assert!(a_binding.is_some(), "Should find 'a' in function scope");
    }

    #[test]
    fn test_scope_tree_shadowing() {
        let code = "let x = 1\nfn foo() {\n  let x = 2\n  return x\n}";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // There should be two bindings named "x" in different scopes
        let x_count: usize = tree
            .scopes
            .iter()
            .flat_map(|s| &s.bindings)
            .filter(|b| b.name == "x")
            .count();
        assert!(
            x_count >= 2,
            "Should have at least 2 bindings named 'x' (shadowing), got {}",
            x_count
        );
    }

    #[test]
    fn test_references_of() {
        let code = "let x = 1\nlet y = x";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // Find the definition of x
        let x_binding = tree.scopes[0].bindings.iter().find(|b| b.name == "x");
        assert!(x_binding.is_some());
        let x = x_binding.unwrap();

        // references_of should return def + references
        let refs = tree.references_of(x.def_span.0);
        assert!(refs.is_some(), "Should find references for x");
        let refs = refs.unwrap();
        assert!(
            refs.len() >= 2,
            "x should have def + at least 1 reference, got {}",
            refs.len()
        );
    }

    #[test]
    fn test_shadowing_does_not_cross_scopes() {
        // Inner x should NOT reference outer x
        let code = "let x = 1\nfn foo() {\n  let x = 2\n  return x\n}\nlet z = x";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // The outer x (at root scope) should have a reference in `z = x`
        // but NOT from the inner `return x`
        let outer_x = tree.scopes[0].bindings.iter().find(|b| b.name == "x");
        assert!(outer_x.is_some(), "Should find outer x");
        let outer_x = outer_x.unwrap();

        // The outer x's references should include `z = x` but not the inner `return x`
        // Since inner x shadows, `return x` binds to inner x, not outer
        let outer_refs = tree.references_of(outer_x.def_span.0).unwrap();
        // Outer x def + reference in `z = x` = 2
        assert_eq!(
            outer_refs.len(),
            2,
            "Outer x should have exactly 2 spans (def + z=x ref), got {}",
            outer_refs.len()
        );
    }

    #[test]
    fn test_definition_of() {
        let code = "let myVar = 42\nlet result = myVar + 1";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // Find the reference to myVar in `myVar + 1`
        let ref_offset = code.rfind("myVar").unwrap();
        let def = tree.definition_of(ref_offset);
        assert!(def.is_some(), "Should find definition of myVar");

        let (start, end) = def.unwrap();
        assert_eq!(
            &code[start..end],
            "myVar",
            "Definition should point to 'myVar'"
        );
    }

    #[test]
    fn test_closure_scope() {
        let code = "let x = 1\nlet f = |y| y + x";
        let program = parse_program(code).unwrap();
        let tree = ScopeTree::build(&program, code);

        // x from the outer scope should be referenced inside the closure
        let outer_x = tree.scopes[0].bindings.iter().find(|b| b.name == "x");
        assert!(outer_x.is_some(), "Should find outer x");
        let outer_x = outer_x.unwrap();
        assert!(
            !outer_x.references.is_empty(),
            "Outer x should be referenced from inside closure"
        );
    }
}
