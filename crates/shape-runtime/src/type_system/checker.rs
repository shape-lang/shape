//! Type Checker
//!
//! Performs type checking on Shape programs using the type inference engine
//! and reports type errors with helpful messages.

use super::errors::{TypeError, TypeErrorWithLocation, TypeResult};
use super::inference::TypeInferenceEngine;
use super::*;
use shape_ast::ast::{EnumDef, Expr, Item, Program, Span, Statement, TypeAnnotation};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeAnalysisMode {
    FailFast,
    RecoverAll,
}

pub struct TypeChecker {
    /// Type inference engine
    inference_engine: TypeInferenceEngine,
    /// Collected errors
    errors: Vec<TypeErrorWithLocation>,
    /// Source code for error reporting
    source: Option<String>,
    /// File name for error reporting
    filename: Option<String>,
    /// Enum definitions for resolving named types
    enum_defs: HashMap<String, EnumDef>,
    /// Current function's parameter types (name -> type annotation)
    current_function_params: HashMap<String, shape_ast::ast::TypeAnnotation>,
    /// Error emission behavior for semantic analysis.
    analysis_mode: TypeAnalysisMode,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        TypeChecker {
            inference_engine: TypeInferenceEngine::new(),
            errors: Vec::new(),
            source: None,
            filename: None,
            enum_defs: HashMap::new(),
            current_function_params: HashMap::new(),
            analysis_mode: TypeAnalysisMode::FailFast,
        }
    }

    /// Set source code for error reporting
    pub fn with_source(mut self, source: String) -> Self {
        self.source = Some(source);
        self
    }

    /// Set filename for error reporting
    pub fn with_filename(mut self, filename: String) -> Self {
        self.filename = Some(filename);
        self
    }

    /// Register host-provided root-scope bindings (e.g. extension module namespaces).
    pub fn with_known_bindings(mut self, names: &[String]) -> Self {
        self.inference_engine.register_known_bindings(names);
        self
    }

    /// WF-3A-tail: register native module exports' declared SCALAR return types
    /// (`"time::millis" -> "number"`) so a module-qualified call infers its true
    /// type in operand / argument / index position. See
    /// `TypeInferenceEngine::module_qualified_scalar_returns`.
    pub fn with_module_qualified_scalar_returns(
        mut self,
        map: std::collections::HashMap<String, String>,
    ) -> Self {
        self.inference_engine
            .register_module_qualified_scalar_returns(map);
        self
    }

    pub fn with_analysis_mode(mut self, mode: TypeAnalysisMode) -> Self {
        self.analysis_mode = mode;
        self
    }

    /// Treat the analyzed program as a compiler-proven comptime mini-program.
    pub fn with_root_comptime_context(mut self, enabled: bool) -> Self {
        self.inference_engine.set_root_comptime_context(enabled);
        self
    }

    /// Type check a complete program
    pub fn check_program(
        &mut self,
        program: &Program,
    ) -> Result<TypeCheckResult, Vec<TypeErrorWithLocation>> {
        // Clear previous errors
        self.errors.clear();
        self.enum_defs.clear();

        // Collect enum definitions for type resolution
        for item in &program.items {
            if let Item::Enum(enum_def, _) = item {
                self.enum_defs
                    .insert(enum_def.name.clone(), enum_def.clone());
            }
        }

        let types = match self.analysis_mode {
            TypeAnalysisMode::FailFast => match self.inference_engine.infer_program(program) {
                Ok(types) => types,
                Err(err) => {
                    self.add_inference_error(err);
                    return Err(self.errors.clone());
                }
            },
            TypeAnalysisMode::RecoverAll => {
                let (types, inference_errors) =
                    self.inference_engine.infer_program_best_effort(program);
                for err in inference_errors {
                    self.add_inference_error(err);
                }
                types
            }
        };

        // Perform additional type checking
        self.check_items(&program.items);

        // Check exhaustiveness of match expressions
        self.check_expressions(&program.items);

        self.prune_error_cascades();

        if self.errors.is_empty() {
            // Convert inference types to semantic types
            let semantic_types: HashMap<String, SemanticType> = types
                .iter()
                .filter_map(|(name, ty)| ty.to_semantic().map(|st| (name.clone(), st)))
                .collect();

            Ok(TypeCheckResult {
                types,
                semantic_types,
                warnings: Vec::new(),
            })
        } else {
            Err(self.errors.clone())
        }
    }

    fn add_inference_error(&mut self, err: TypeError) {
        let (line, col) = self.find_inference_error_position(&err);
        self.add_error(err, line, col);
    }

    fn prune_error_cascades(&mut self) {
        let has_specific_errors = self
            .errors
            .iter()
            .any(|err| !matches!(err.error, TypeError::UnsolvedConstraints(_)));
        if has_specific_errors {
            self.errors
                .retain(|err| !matches!(err.error, TypeError::UnsolvedConstraints(_)));
        }

        let mut seen = HashSet::new();
        self.errors.retain(|err| {
            let key = (err.line, err.column, err.error.to_string());
            seen.insert(key)
        });
    }

    fn find_inference_error_position(&self, error: &TypeError) -> (usize, usize) {
        match error {
            TypeError::UnknownProperty(_, property) => {
                if let Some(span) = self
                    .inference_engine
                    .lookup_unknown_property_origin(property)
                {
                    if let Some((line, col)) = self.span_to_line_col(span) {
                        return (line, col);
                    }
                }
                (0, 0)
            }
            TypeError::UndefinedVariable(name) => self
                .inference_engine
                .lookup_undefined_variable_origin(name)
                .and_then(|span| self.span_to_line_col(span))
                .unwrap_or((0, 0)),
            TypeError::UnsolvedConstraints(constraints) => {
                if let Some(span) = self
                    .inference_engine
                    .find_origin_for_unsolved_constraints(constraints)
                {
                    if let Some((line, col)) = self.span_to_line_col(span) {
                        return (line, col);
                    }
                }
                if let Some(span) = self.inference_engine.find_any_constraint_origin() {
                    if let Some((line, col)) = self.span_to_line_col(span) {
                        return (line, col);
                    }
                }
                (0, 0)
            }
            TypeError::InvalidAssertion(_, _) => (0, 0),
            TypeError::NonExhaustiveMatch { enum_name, .. } => self
                .inference_engine
                .lookup_non_exhaustive_match_origin(enum_name)
                .and_then(|span| self.span_to_line_col(span))
                .unwrap_or((0, 0)),
            TypeError::GenericTypeError { symbol, .. } => {
                if let Some(symbol) = symbol
                    && let Some(span) = self
                        .inference_engine
                        .lookup_callable_origin_for_name(symbol)
                    && let Some((line, col)) = self.span_to_line_col(span)
                {
                    return (line, col);
                }
                if let Some(span) = self.inference_engine.find_any_constraint_origin() {
                    if let Some((line, col)) = self.span_to_line_col(span) {
                        return (line, col);
                    }
                }
                (0, 0)
            }
            _ => (0, 0),
        }
    }

    fn span_to_line_col(&self, span: shape_ast::ast::Span) -> Option<(usize, usize)> {
        let source = self.source.as_ref()?;
        let start = span.start.min(source.len());
        let prefix = &source[..start];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        let column = prefix[line_start..].chars().count() + 1;
        Some((line, column))
    }

    /// Check all items in the program
    fn check_items(&mut self, items: &[Item]) {
        for item in items {
            self.check_item(item);
        }
    }

    /// Check all expressions in the program for exhaustiveness
    fn check_expressions(&mut self, items: &[Item]) {
        for item in items {
            self.check_item_expressions(item);
        }
    }

    /// Check expressions within an item
    fn check_item_expressions(&mut self, item: &Item) {
        if let Item::Function(func, _) = item {
            // Set up function parameter context for type resolution
            self.current_function_params.clear();
            for param in &func.params {
                if let Some(type_ann) = &param.type_annotation {
                    // Insert all identifiers from the pattern
                    for name in param.get_identifiers() {
                        self.current_function_params.insert(name, type_ann.clone());
                    }
                }
            }

            for stmt in &func.body {
                self.check_statement_expressions(stmt);
            }

            // Clear function parameter context
            self.current_function_params.clear();
        }
    }

    /// Check expressions within a statement
    fn check_statement_expressions(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Expression(expr, _) => self.check_expr(expr),
            Statement::Return(Some(expr), _) => self.check_expr(expr),
            Statement::VariableDecl(decl, _) => {
                if let Some(init) = &decl.value {
                    self.check_expr(init);
                }
            }
            Statement::If(if_stmt, _) => {
                self.check_expr(&if_stmt.condition);
                for stmt in &if_stmt.then_body {
                    self.check_statement_expressions(stmt);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        self.check_statement_expressions(stmt);
                    }
                }
            }
            Statement::While(while_loop, _) => {
                self.check_expr(&while_loop.condition);
                for stmt in &while_loop.body {
                    self.check_statement_expressions(stmt);
                }
            }
            Statement::For(for_loop, _) => {
                for stmt in &for_loop.body {
                    self.check_statement_expressions(stmt);
                }
            }
            _ => {}
        }
    }

    /// Check a single expression for type issues
    ///
    /// Note: Exhaustiveness checking is now handled by the inference engine
    /// during `infer_expr()`, so we don't need to check it here.
    fn check_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Match(match_expr, _span) => {
                // Exhaustiveness is checked by inference engine - just check sub-expressions
                self.check_expr(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard);
                    }
                    self.check_expr(&arm.body);
                }
            }
            // Recursively check sub-expressions
            Expr::BinaryOp { left, right, .. } => {
                self.check_expr(left);
                self.check_expr(right);
            }
            Expr::UnaryOp { operand, .. } => {
                self.check_expr(operand);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.check_expr(condition);
                self.check_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.check_expr(else_e);
                }
            }
            Expr::If(if_expr, _) => {
                self.check_expr(&if_expr.condition);
                self.check_expr(&if_expr.then_branch);
                if let Some(else_branch) = &if_expr.else_branch {
                    self.check_expr(else_branch);
                }
            }
            Expr::FunctionCall {
                const_args, args, ..
            } => {
                for arg in const_args {
                    self.check_expr(arg);
                }
                for arg in args {
                    self.check_expr(arg);
                }
            }
            Expr::QualifiedFunctionCall {
                const_args, args, ..
            } => {
                for arg in const_args {
                    self.check_expr(arg);
                }
                for arg in args {
                    self.check_expr(arg);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            Expr::Array(elems, _) => {
                for elem in elems {
                    self.check_expr(elem);
                }
            }
            Expr::PropertyAccess { object, .. } => {
                self.check_expr(object);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.check_expr(object);
                self.check_expr(index);
                if let Some(end) = end_index {
                    self.check_expr(end);
                }
            }
            _ => {}
        }
    }

    // Note: check_match_exhaustiveness, resolve_named_to_enum, and span_to_location
    // were removed as exhaustiveness checking is now handled by the inference engine
    // in TypeInferenceEngine::infer_expr() for Match expressions.

    /// Check a single item
    fn check_item(&mut self, item: &Item) {
        match item {
            Item::Function(func, span) => {
                // Check for missing return statements
                if func.return_type.is_some()
                    && !matches!(func.return_type.as_ref().unwrap(), TypeAnnotation::Void)
                    && !self.has_return_statement(&func.body)
                {
                    let (line, col) = self.item_span_to_line_col(*span);
                    self.add_error(TypeError::MissingReturn(func.name.clone()), line, col);
                }
            }

            Item::TypeAlias(alias, span) => {
                // Check for cyclic type aliases
                if self.is_cyclic_type_alias(&alias.name, &alias.type_annotation) {
                    let (line, col) = self.item_span_to_line_col(*span);
                    self.add_error(TypeError::CyclicTypeAlias(alias.name.clone()), line, col);
                }
            }

            _ => {}
        }
    }

    /// Check if statements contain a return statement
    fn has_return_statement(&self, stmts: &[Statement]) -> bool {
        // A value-producing TAIL EXPRESSION is the implicit return in Shape's
        // tail-expression semantics: `fn f() -> int { 42 }`,
        // `fn g(c: Color) -> string { match c { ... } }`, and
        // `fn h(b: bool) -> int { if b { 1 } else { 2 } }` all return a value via
        // their final expression — there is no explicit `return` keyword. The
        // last statement being a `Statement::Expression` (or a value-yielding
        // tail `if`/`match`) therefore satisfies the requirement. Type agreement
        // between the tail expression and the declared return type is enforced
        // separately by inference; this check only asks "is there a value-
        // producing terminal at all".
        if Self::block_yields_tail_value(stmts) {
            return true;
        }

        for stmt in stmts {
            match stmt {
                Statement::Return(_, _) => return true,
                Statement::If(if_stmt, _) => {
                    // Both branches must have returns
                    if let Some(else_body) = &if_stmt.else_body {
                        if self.has_return_statement(&if_stmt.then_body)
                            && self.has_return_statement(else_body)
                        {
                            return true;
                        }
                    }
                }
                Statement::While(while_loop, _) => {
                    if self.has_return_statement(&while_loop.body) {
                        // Note: This is conservative - while loop might not execute
                        return true;
                    }
                }
                Statement::For(for_loop, _) => {
                    if self.has_return_statement(&for_loop.body) {
                        // Note: This is conservative - for loop might not execute
                        return true;
                    }
                }
                _ => {}
            }
        }

        false
    }

    /// True when the final statement of a function body produces a value (the
    /// implicit tail return). Recurses through tail `if`/`match`/block so that
    /// `if cond { a } else { b }` and `match x { ... }` count when every arm
    /// yields a value via its own tail expression.
    fn block_yields_tail_value(stmts: &[Statement]) -> bool {
        let Some(last) = stmts.last() else {
            return false;
        };
        match last {
            // An explicit `return` is a value-producing terminal.
            Statement::Return(_, _) => true,
            // A bare tail expression is the implicit return value.
            Statement::Expression(expr, _) => Self::expr_yields_tail_value(expr),
            _ => false,
        }
    }

    /// True when the items of a block expression (`{ ... }`) end in a
    /// value-producing tail. The trailing `BlockItem::Expression` carries the
    /// block's value; an explicit `return` also terminates with a value.
    fn block_items_yield_tail_value(items: &[shape_ast::ast::BlockItem]) -> bool {
        use shape_ast::ast::BlockItem;
        match items.last() {
            Some(BlockItem::Expression(expr)) => Self::expr_yields_tail_value(expr),
            Some(BlockItem::Statement(Statement::Return(_, _))) => true,
            Some(BlockItem::Statement(Statement::Expression(expr, _))) => {
                Self::expr_yields_tail_value(expr)
            }
            _ => false,
        }
    }

    /// True when an expression in tail position produces a value for the
    /// enclosing function's implicit return. Plain value expressions (literals,
    /// calls, arithmetic, identifiers, ...) always do; tail `if`/`match`/`block`
    /// produce a value only when every arm/branch does.
    fn expr_yields_tail_value(expr: &Expr) -> bool {
        match expr {
            // `if` in tail position yields a value only if BOTH branches do; a
            // one-armed `if` (no `else`) cannot be the function's value. The
            // parser emits `Expr::Conditional`; a desugar pass may also produce
            // `Expr::If` — handle both shapes.
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                let Some(else_expr) = else_expr else {
                    return false;
                };
                Self::expr_yields_tail_value(then_expr) && Self::expr_yields_tail_value(else_expr)
            }
            Expr::If(if_expr, _) => {
                let Some(else_branch) = &if_expr.else_branch else {
                    return false;
                };
                Self::expr_yields_tail_value(&if_expr.then_branch)
                    && Self::expr_yields_tail_value(else_branch)
            }
            // `match` yields a value when every arm's body yields one. An empty
            // match (no arms) cannot produce a value.
            Expr::Match(match_expr, _) => {
                !match_expr.arms.is_empty()
                    && match_expr
                        .arms
                        .iter()
                        .all(|arm| Self::expr_yields_tail_value(&arm.body))
            }
            // A nested block yields a value when its own tail does.
            Expr::Block(block_expr, _) => Self::block_items_yield_tail_value(&block_expr.items),
            // An explicit `return` expression always produces a value-terminal.
            Expr::Return(_, _) => true,
            // `Unit` (void) is not a value for a non-void return type. A tail
            // `if` whose synthesized else is Unit must NOT satisfy the check.
            Expr::Unit(_) => false,
            // Any other expression in tail position is a value (literal, call,
            // binary op, identifier, constructor, ...). Type correctness against
            // the declared return type is checked by inference, not here.
            _ => true,
        }
    }

    /// Check for cyclic type aliases
    fn is_cyclic_type_alias(&self, name: &str, ty: &TypeAnnotation) -> bool {
        self.references_type(ty, name)
    }

    /// Check if a type annotation references a specific type name
    fn references_type(&self, ty: &TypeAnnotation, name: &str) -> bool {
        match ty {
            TypeAnnotation::Reference(ref_name) => ref_name == name,
            TypeAnnotation::Array(elem) => self.references_type(elem, name),
            TypeAnnotation::Tuple(elems) => {
                elems.iter().any(|elem| self.references_type(elem, name))
            }
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| self.references_type(&field.type_annotation, name)),
            TypeAnnotation::Function {
                params, returns, ..
            } => {
                params
                    .iter()
                    .any(|param| self.references_type(&param.type_annotation, name))
                    || self.references_type(returns, name)
            }
            TypeAnnotation::Union(types) => types.iter().any(|ty| self.references_type(ty, name)),
            TypeAnnotation::Generic { args, .. } => {
                args.iter().any(|arg| self.references_type(arg, name))
            }
            _ => false,
        }
    }

    fn item_span_to_line_col(&self, span: Span) -> (usize, usize) {
        self.span_to_line_col(span).unwrap_or((0, 0))
    }

    /// Add an error with location information
    fn add_error(&mut self, error: TypeError, line: usize, column: usize) {
        let fixes = self.machine_applicable_fixes(&error);
        let mut err = TypeErrorWithLocation::new(error, line, column);

        if let Some(filename) = &self.filename {
            err = err.with_file(filename.clone());
        }

        if let Some(source) = &self.source {
            // Extract the source line
            if let Some(source_line) = source.lines().nth(line.saturating_sub(1)) {
                err = err.with_source_line(source_line.to_string());
            }
        }

        self.errors.push(err.with_fixes(fixes));
    }

    /// Fixes the checker can prove for `error` (ADR-017 §4).
    ///
    /// Every consumer reads what this returns; nothing downstream re-derives
    /// a fix from the rendered message. Errors with no proved fix return
    /// empty, which is not the same as "the tool should guess".
    fn machine_applicable_fixes(&self, error: &TypeError) -> Vec<shape_diagnostics::SuggestedFix> {
        let Some(source) = self.source.as_deref() else {
            return Vec::new();
        };

        match error {
            TypeError::NonExhaustiveMatch {
                enum_name,
                missing_variants,
            } => self
                .inference_engine
                .lookup_non_exhaustive_match_origin(enum_name)
                .and_then(|span| {
                    super::fixes::non_exhaustive_match_fix(
                        source,
                        span,
                        enum_name,
                        missing_variants,
                    )
                })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Get all collected errors
    pub fn errors(&self) -> &[TypeErrorWithLocation] {
        &self.errors
    }

    /// Format all errors for display
    pub fn format_errors(&self) -> String {
        self.errors
            .iter()
            .map(|err| err.format_with_source())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Shared single-entry type analysis used by compiler and LSP.
pub fn analyze_program(
    program: &Program,
    source: Option<&str>,
    filename: Option<&str>,
    known_bindings: Option<&[String]>,
) -> Result<TypeCheckResult, Vec<TypeErrorWithLocation>> {
    analyze_program_with_mode(
        program,
        source,
        filename,
        known_bindings,
        TypeAnalysisMode::FailFast,
    )
}

/// Shared type analysis with explicit recovery behavior.
pub fn analyze_program_with_mode(
    program: &Program,
    source: Option<&str>,
    filename: Option<&str>,
    known_bindings: Option<&[String]>,
    analysis_mode: TypeAnalysisMode,
) -> Result<TypeCheckResult, Vec<TypeErrorWithLocation>> {
    analyze_program_with_mode_and_comptime_context(
        program,
        source,
        filename,
        known_bindings,
        analysis_mode,
        false,
    )
}

/// Shared type analysis with explicit recovery behavior and root comptime proof.
pub fn analyze_program_with_mode_and_comptime_context(
    program: &Program,
    source: Option<&str>,
    filename: Option<&str>,
    known_bindings: Option<&[String]>,
    analysis_mode: TypeAnalysisMode,
    root_comptime_context: bool,
) -> Result<TypeCheckResult, Vec<TypeErrorWithLocation>> {
    analyze_program_full(
        program,
        source,
        filename,
        known_bindings,
        analysis_mode,
        root_comptime_context,
        None,
    )
}

/// WF-3A-tail: full analysis entry that additionally accepts the native
/// module-export SCALAR return-type map (`"time::millis" -> "number"`) so a
/// module-qualified call carries its declared type in operand / argument /
/// index position (not only when bound to a `let`). The map is scalar-only;
/// `Result<..>`/`Option<..>`/heap returns are omitted by the compiler so the
/// existing json/msgpack navigation path is untouched.
#[allow(clippy::too_many_arguments)]
pub fn analyze_program_full(
    program: &Program,
    source: Option<&str>,
    filename: Option<&str>,
    known_bindings: Option<&[String]>,
    analysis_mode: TypeAnalysisMode,
    root_comptime_context: bool,
    module_qualified_scalar_returns: Option<HashMap<String, String>>,
) -> Result<TypeCheckResult, Vec<TypeErrorWithLocation>> {
    let mut checker = TypeChecker::new();
    if let Some(src) = source {
        checker = checker.with_source(src.to_string());
    }
    if let Some(file) = filename {
        checker = checker.with_filename(file.to_string());
    }
    if let Some(names) = known_bindings {
        checker = checker.with_known_bindings(names);
    }
    if let Some(map) = module_qualified_scalar_returns {
        checker = checker.with_module_qualified_scalar_returns(map);
    }
    checker = checker.with_root_comptime_context(root_comptime_context);
    checker = checker.with_analysis_mode(analysis_mode);
    checker.check_program(program)
}

/// Result of type checking
#[derive(Debug)]
pub struct TypeCheckResult {
    /// Inferred types for all declarations (inference-level types)
    pub types: HashMap<String, Type>,
    /// Semantic types for all declarations (user-facing types)
    pub semantic_types: HashMap<String, SemanticType>,
    /// Type warnings (non-fatal issues)
    pub warnings: Vec<TypeWarning>,
}

impl TypeCheckResult {
    /// Get the semantic type for a declaration
    pub fn get_semantic_type(&self, name: &str) -> Option<&SemanticType> {
        self.semantic_types.get(name)
    }

    /// Get all function declarations that are fallible (return Result)
    pub fn fallible_functions(&self) -> Vec<&str> {
        self.semantic_types
            .iter()
            .filter_map(|(name, ty)| {
                if let SemanticType::Function(sig) = ty {
                    if sig.return_type.is_result() {
                        return Some(name.as_str());
                    }
                }
                None
            })
            .collect()
    }
}

/// Type warning for non-fatal issues
#[derive(Debug)]
pub struct TypeWarning {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// Type check an expression and return its type
pub fn type_of_expr(expr: &Expr, _env: &TypeEnvironment) -> TypeResult<Type> {
    let mut engine = TypeInferenceEngine::new();
    engine.infer_expr(expr)
}

/// Quick type check for REPL and testing
pub fn quick_check(source: &str) -> Result<TypeCheckResult, String> {
    use shape_ast::parser::parse_program;

    let program = parse_program(source).map_err(|e| format!("Parse error: {}", e))?;

    let mut checker = TypeChecker::new().with_source(source.to_string());

    checker.check_program(&program).map_err(|errors| {
        errors
            .iter()
            .map(|e| e.format_with_source())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_comptime_context_allows_comptime_builtin_forwarder() {
        let source = r#"
            fn build_config() -> int { 1 }
            build_config()
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");

        let outside = analyze_program_with_mode(
            &program,
            Some(source),
            None,
            None,
            TypeAnalysisMode::FailFast,
        );
        assert!(
            format!("{:?}", outside.err().expect("outside comptime must fail"))
                .contains("comptime-only builtin"),
            "ordinary source must still reject comptime-only builtin calls"
        );

        let inside = analyze_program_with_mode_and_comptime_context(
            &program,
            Some(source),
            None,
            None,
            TypeAnalysisMode::FailFast,
            true,
        );
        assert!(
            inside.is_ok(),
            "compiler-proven comptime mini-program should type-check: {:?}",
            inside.err()
        );
    }

    #[test]
    fn w83e_comptime_fn_body_allows_comptime_only_builtin_gate() {
        let source = r#"
            comptime fn require_const_host() {
                if false {
                    error("not executed")
                }
            }

            comptime {
                require_const_host()
            }
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");

        let result = analyze_program_with_mode(
            &program,
            Some(source),
            None,
            None,
            TypeAnalysisMode::FailFast,
        );
        assert!(
            result.is_ok(),
            "comptime fn bodies are comptime contexts; got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_exhaustiveness_integration_non_exhaustive_match_produces_error() {
        // This test proves exhaustiveness checking is connected to the compiler pipeline.
        // A match on an enum that doesn't cover all variants should produce an error.
        let source = r#"
            enum Status { Active, Inactive, Pending }

            function check(s: Status) {
                return match s {
                    Status::Active => "yes"
                };
            }
        "#;

        let result = quick_check(source);

        // The match is non-exhaustive (missing Inactive and Pending)
        // so we expect an error
        assert!(
            result.is_err(),
            "Expected error for non-exhaustive match, got: {:?}",
            result
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("NonExhaustive")
                || err.contains("non-exhaustive")
                || err.contains("missing"),
            "Expected non-exhaustive match error, got: {}",
            err
        );
    }

    /// ADR-017 §4: the checker hands out the fix as proved facts, and
    /// applying it produces source that type-checks clean. The fix's
    /// insertion point comes from the match's own span, so this also pins
    /// that the span reaches through the closing brace.
    #[test]
    fn non_exhaustive_match_carries_an_applicable_fix() {
        let source = "enum Status { Active, Inactive, Pending }\n\
                      fn check(s: Status) -> string {\n\
                      \x20 match s {\n\
                      \x20   Status::Active => \"yes\",\n\
                      \x20 }\n\
                      }\n";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let errors = analyze_program(&program, Some(source), None, None)
            .expect_err("non-exhaustive match must fail");

        let non_exhaustive = errors
            .iter()
            .find(|e| matches!(e.error, TypeError::NonExhaustiveMatch { .. }))
            .expect("NonExhaustiveMatch reported");

        assert_eq!(
            non_exhaustive.fixes.len(),
            1,
            "expected exactly one proved fix, got {:?}",
            non_exhaustive.fixes
        );
        let fix = &non_exhaustive.fixes[0];
        assert_eq!(fix.label, "Add missing match arms for Status");

        let repaired = fix
            .edit_plan
            .as_ref()
            .expect("machine-applicable")
            .apply(source)
            .expect("plan applies to the source it was proved against");
        assert_eq!(
            repaired,
            "enum Status { Active, Inactive, Pending }\n\
             fn check(s: Status) -> string {\n\
             \x20 match s {\n\
             \x20   Status::Active => \"yes\",\n\
             \x20   Status::Inactive => {\n\
             \x20   },\n\
             \x20   Status::Pending => {\n\
             \x20   },\n\
             \x20 }\n\
             }\n"
        );

        let repaired_program = shape_ast::parser::parse_program(&repaired).expect("fix parses");
        let remaining = analyze_program(&repaired_program, Some(&repaired), None, None).err();
        assert!(
            !remaining
                .iter()
                .flatten()
                .any(|e| matches!(e.error, TypeError::NonExhaustiveMatch { .. })),
            "the applied fix must retire the diagnostic it fixes, got: {remaining:?}"
        );
    }

    /// The same input must produce the same edit bytes, every compile.
    ///
    /// A machine-applicable fix that varies run to run is not evidence, and
    /// this varied: the missing-variant list came from a hash-set difference,
    /// so the arms were emitted in a different order from process to process.
    /// Repeating inside one process is a real test of the property — two
    /// `HashSet`s in the same thread do not share an iteration order — and
    /// the expected value pins declaration order specifically.
    #[test]
    fn the_proved_fix_is_byte_identical_across_compiles() {
        let source = "enum Status { Active, Inactive, Pending, Archived }\n\
                      fn check(s: Status) -> string {\n\
                      \x20 match s {\n\
                      \x20   Status::Active => \"yes\",\n\
                      \x20 }\n\
                      }\n";

        let edit_bytes = || {
            let program = shape_ast::parser::parse_program(source).expect("parse");
            let errors = analyze_program(&program, Some(source), None, None)
                .expect_err("non-exhaustive match must fail");
            errors
                .iter()
                .find(|e| matches!(e.error, TypeError::NonExhaustiveMatch { .. }))
                .expect("NonExhaustiveMatch reported")
                .fixes
                .first()
                .expect("a proved fix")
                .edit_plan
                .as_ref()
                .expect("machine-applicable")
                .edits[0]
                .new_text
                .clone()
        };

        let first = edit_bytes();
        assert_eq!(
            first,
            "    Status::Inactive => {\n    },\n\
             \x20   Status::Pending => {\n    },\n\
             \x20   Status::Archived => {\n    },\n",
            "arms must materialize in enum declaration order"
        );
        for _ in 0..16 {
            assert_eq!(
                edit_bytes(),
                first,
                "the same input must give the same edit"
            );
        }
    }

    /// A checker with no source cannot bind a plan to a revision, so it
    /// proves no machine-applicable fix rather than emitting an unbound one.
    #[test]
    fn non_exhaustive_match_without_source_proves_no_fix() {
        let source = "enum Status { Active, Inactive }\n\
                      fn check(s: Status) -> string {\n\
                      \x20 match s {\n\
                      \x20   Status::Active => \"yes\",\n\
                      \x20 }\n\
                      }\n";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let errors = analyze_program(&program, None, None, None).expect_err("must fail");
        assert!(errors.iter().all(|e| e.fixes.is_empty()));
    }

    #[test]
    fn test_exhaustiveness_integration_exhaustive_match_succeeds() {
        // A match that covers all variants should succeed
        let source = r#"
            enum Status { Active, Inactive }

            function check(s: Status) {
                return match s {
                    Status::Active => "yes",
                    Status::Inactive => "no"
                };
            }
        "#;

        let result = quick_check(source);

        // The match is exhaustive, so no error expected from exhaustiveness
        // (there might be other errors, but not NonExhaustiveMatch)
        if let Err(err) = &result {
            assert!(
                !err.contains("NonExhaustive") && !err.contains("non-exhaustive"),
                "Should not have non-exhaustive error for exhaustive match, got: {}",
                err
            );
        }
    }

    #[test]
    fn test_exhaustiveness_integration_wildcard_makes_exhaustive() {
        // A match with wildcard pattern should be trivially exhaustive
        let source = r#"
            enum Status { Active, Inactive, Pending }

            function check(s: Status) {
                return match s {
                    Status::Active => "yes",
                    _ => "other"
                };
            }
        "#;

        let result = quick_check(source);

        // The wildcard makes it exhaustive
        if let Err(err) = &result {
            assert!(
                !err.contains("NonExhaustive") && !err.contains("non-exhaustive"),
                "Wildcard should make match exhaustive, got: {}",
                err
            );
        }
    }

    #[test]
    fn test_undefined_variable_reports_identifier_position() {
        use shape_ast::parser::parse_program;

        let source = r#"
let x = 1
let y = duckdb.connect("duckdb://analytics.db")
"#;

        let program = parse_program(source).expect("program should parse");
        let result = analyze_program(&program, Some(source), None, None);
        let errors = result.expect_err("undefined variable should fail analysis");
        let undef = errors
            .iter()
            .find(|e| matches!(&e.error, TypeError::UndefinedVariable(name) if name == "duckdb"))
            .expect("missing undefined-variable error for duckdb");

        assert_eq!(undef.line, 3);
        assert_eq!(undef.column, 9);
    }

    #[test]
    fn test_known_bindings_allow_extension_namespace_in_type_analysis() {
        use shape_ast::parser::parse_program;

        let source = r#"let conn = duckdb.connect("duckdb://analytics.db")"#;
        let program = parse_program(source).expect("program should parse");
        let known = vec!["duckdb".to_string()];

        let result = analyze_program(&program, Some(source), None, Some(&known));
        assert!(
            result.is_ok(),
            "known extension namespaces should not fail type analysis: {:?}",
            result.err()
        );
    }
}
