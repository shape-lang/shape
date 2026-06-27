//! Statement-level type inference
//!
//! Handles type inference for statements: if, while, for, return, etc.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Assignment, BinaryOp, Expr, Literal, Span, Statement, TypeAnnotation};

impl TypeInferenceEngine {
    /// Infer an assignment to an existing binding (`name = expr` or a
    /// destructuring `(a, b) = expr`).
    ///
    /// Shared by `Statement::Assignment` and `BlockItem::Assignment` — a
    /// for/while loop body is parsed as an `Expr::Block` whose items are
    /// `BlockItem::Assignment`, so the RHS of a loop-body assignment must be
    /// inferred here too. Skipping it severs the call graph: the RHS of
    /// `last = dbl(11)` inside a loop would otherwise never be walked, so the
    /// callsite of `dbl` is never recorded and an unannotated parameter
    /// collapses to the `number` default — producing a kind-confused result.
    pub(crate) fn infer_assignment(&mut self, assign: &Assignment, span: Span) -> TypeResult<()> {
        let value_type = self.infer_expr(&assign.value)?;
        if let Some(name) = assign.pattern.as_identifier() {
            let scheme = self.env.lookup(name).cloned();
            let target_type = match scheme {
                Some(s) => s.instantiate(&mut self.type_var_gen),
                None => {
                    self.register_undefined_variable_origin(name, span);
                    return Err(TypeError::UndefinedVariable(name.to_string()));
                }
            };
            // Numeric-conversion §4 / §5: a reassignment is a value-level flow
            // `target = value`, so the §2 lattice constraint is the directional
            // `(src=value, dst=target)` — NOT `(target, value)` (the prior
            // ordering wrongly accepted `let mut x:u8=…; x = big_u16` because
            // `u8` widens to the value's `u16`). A bare int-literal RHS adopts
            // the target type when it losslessly fits (`let mut x:u8=0; x=200`),
            // and is REJECTED when out of range (`x = 300` -> the natural-`int`
            // literal fails `lossless_implicit(int, u8)`); a non-literal value
            // follows the value-level lattice directly.
            // ROOT-B narrowing (STAGE-2 soundness regression close): a MUTABLE
            // unannotated `let mut x = <int literal>` binding DEFERS its seed to
            // a fresh var (see `items.rs`) so an accumulator (`sum = sum + v`,
            // `v: number`) can adopt `number`. But that open var must NOT absorb
            // a NON-numeric reassignment (`x = None`, `x = "hello"`): the int
            // literal's family is numeric, so a non-numeric RHS is the same
            // value-level mismatch main rejects (`int := None`). When the target
            // resolves to such a deferred-literal var AND the assigned value
            // resolves to a definitely-non-numeric type, ground the var to its
            // NATURAL `int` type FIRST so the constraint below surfaces the
            // mismatch (`Option<T> !~ int`). A numeric RHS (`sum + v`) or an
            // as-yet-unresolved var RHS is left alone — ROOT-B keeps adopting.
            let mut grounded_seed_mismatch = false;
            if let Type::Variable(target_var) =
                self.solver.unifier().apply_substitutions(&target_type)
            {
                if self
                    .deferred_constructor_literal_payload_vars
                    .contains(&target_var)
                {
                    let resolved_value = self.solver.unifier().apply_substitutions(&value_type);
                    if Self::is_definitely_non_numeric(&resolved_value) {
                        // Ground the seed to its NATURAL `int` and surface the
                        // mismatch directly as `value ~ int` (a clean
                        // "<value> is not compatible with int" render). The
                        // constraint SOLVER owns a separate unifier, so binding
                        // `self.unifier` here would be invisible to it — we push
                        // the grounding constraint AND the explicit
                        // `value ~ int` mismatch, and skip the usual
                        // `value ~ target` constraint (its var-side would render
                        // as the still-unresolved "unknown"). Matches main's
                        // `int := None` / `int := string` rejection.
                        self.constraints
                            .push((value_type.clone(), BuiltinTypes::integer()));
                        self.constraints
                            .push((Type::Variable(target_var), BuiltinTypes::integer()));
                        grounded_seed_mismatch = true;
                    }
                }
            }
            if grounded_seed_mismatch {
                // mismatch constraint already pushed above.
            } else if Self::adopt_int_literal_in_context(&assign.value, &target_type).is_some() {
                // literal fits the target — no rejecting constraint.
            } else {
                self.constraints.push((value_type, target_type));
            }
        } else {
            // Destructuring assignment: conservatively constrain each bound name
            // to the assigned value until full pattern assignment inference lands.
            for name in assign.pattern.get_identifiers() {
                let scheme = self.env.lookup(&name).cloned();
                let target_type = match scheme {
                    Some(s) => s.instantiate(&mut self.type_var_gen),
                    None => {
                        self.register_undefined_variable_origin(&name, span);
                        return Err(TypeError::UndefinedVariable(name.clone()));
                    }
                };
                self.constraints.push((target_type, value_type.clone()));
            }
        }
        Ok(())
    }

    /// Whether a (substitution-resolved) type is DEFINITELY non-numeric — i.e.
    /// a concrete non-numeric basic type (`string`, `bool`, `Option`/`Result`
    /// carriers as a bare name, …) or any generic instantiation (`Option<T>`,
    /// `Result<T>`, `Array<T>`). Used by the ROOT-B reassignment narrowing to
    /// decide whether a deferred int-literal seed var should be grounded to
    /// `int` (surfacing a mismatch) before a non-numeric RHS would otherwise
    /// silently bind it. CONSERVATIVE: an unresolved `Type::Variable`, a
    /// `Constrained` var, a `Function`, or a numeric concrete returns `false`
    /// (the var keeps deferring), so ROOT-B's numeric accumulation is never
    /// disturbed — only a committed non-numeric RHS grounds the seed.
    fn is_definitely_non_numeric(ty: &Type) -> bool {
        match ty {
            Type::Concrete(ann) => ann
                .as_type_name_str()
                .is_some_and(|n| !BuiltinTypes::is_numeric_type_name(n)),
            // Any generic instantiation (Option<_>, Result<_>, Array<_>, …) is a
            // non-numeric carrier — None infers `Option<T>`, a string→`string`.
            Type::Generic { .. } => true,
            _ => false,
        }
    }

    /// ROOT-1 (strict-flip, 2026-06-18): bind a statement-form for-in
    /// (`DestructurePattern`) loop's binders, typing an OBJECT destructure's
    /// fields from the element struct's declared field annotations. Mirror of
    /// the `Expr::For` arm's object-destructure handling (which operates on the
    /// expression-form `Pattern`). A non-object pattern, or a field with no
    /// resolvable type, binds the WHOLE element type (parity preserved).
    fn bind_for_in_destructure_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
        element_type: &Type,
    ) {
        if let shape_ast::ast::DestructurePattern::Object(fields) = pattern {
            let resolved_elem = self.solver.unifier().apply_substitutions(element_type);
            if let Some(struct_name) = self
                .struct_name_of_type(&resolved_elem)
                .or_else(|| self.struct_name_of_type(element_type))
            {
                for field in fields {
                    let binder = field.pattern.as_identifier().unwrap_or(&field.key);
                    let field_ty = self
                        .struct_field_annotation(&struct_name, &field.key)
                        .map(|ann| self.resolve_type_annotation(&ann))
                        .unwrap_or_else(|| element_type.clone());
                    self.env.define(binder, TypeScheme::mono(field_ty));
                }
                return;
            }
            // T1 sub-case (d) (strict-flip, 2026-06-20): anonymous object-literal
            // element (`for {x, y} in [{x: 1, y: 2}]`) has no registered struct
            // name. Bind each destructured field from the element's own recorded
            // field annotation (object-literal inference already froze the field
            // types). Mirror of the `Expr::For` arm. PER-SITE-ARM, no fabrication.
            if let Type::Concrete(TypeAnnotation::Object(elem_fields)) = &resolved_elem {
                for field in fields {
                    let binder = field.pattern.as_identifier().unwrap_or(&field.key);
                    let field_ty = elem_fields
                        .iter()
                        .find(|f| f.name == field.key)
                        .map(|f| self.resolve_type_annotation(&f.type_annotation))
                        .unwrap_or_else(|| element_type.clone());
                    self.env.define(binder, TypeScheme::mono(field_ty));
                }
                return;
            }
        }
        for name in pattern.get_identifiers() {
            self.env
                .define(&name, TypeScheme::mono(element_type.clone()));
        }
    }

    /// Infer type of statements
    pub(crate) fn infer_statements(&mut self, stmts: &[Statement]) -> TypeResult<Type> {
        let mut last_type = BuiltinTypes::void();

        for stmt in stmts {
            last_type = self.infer_statement(stmt)?;
        }

        Ok(last_type)
    }

    /// Infer a callable body, applying numeric-conversion §4 literal adoption to
    /// the TAIL expression statement when the enclosing fn declares a numeric or
    /// Result/Option return type. Only the final expression-style statement is
    /// checked against the expected return type (a non-tail expression statement
    /// keeps plain inference, so `foo(); 42` does not wrongly constrain
    /// `foo()`). A bare int literal tail, or the int-literal arms of a tail
    /// `match`/`if`, adopt the declared numeric return type via `check_against`.
    ///
    /// For a Result/Option declared return the tail is checked against the
    /// carrier ONLY when it is a matching `Ok`/`Err`/`Some` / user-enum
    /// constructor — so the expected variant payload reaches the constructor's
    /// argument (`fn f() -> Result<number> { Ok(42) }`). A bare-value tail
    /// (`{ 42 }`) is LEFT to plain inference so Shape's implicit `Ok`/`Some`-wrap
    /// (`push_return_constraint`) still applies; constraining a bare `int` tail
    /// against `Result<number>` would wrongly reject the implicit wrap.
    fn infer_statements_with_return_adoption(&mut self, stmts: &[Statement]) -> TypeResult<Type> {
        let expected = match self.expected_return_types.last().cloned() {
            Some(Some(ty)) => ty,
            _ => return self.infer_statements(stmts),
        };
        let expected_is_carrier = self.is_result_type(&expected) || self.is_option_type(&expected);
        let mut last_type = BuiltinTypes::void();
        let n = stmts.len();
        for (idx, stmt) in stmts.iter().enumerate() {
            let is_tail = idx + 1 == n;
            if is_tail {
                match stmt {
                    Statement::Expression(expr, _) => {
                        // For a Result/Option carrier, only a matching
                        // constructor tail goes through `check_against` (payload
                        // propagation); everything else keeps plain inference +
                        // implicit-wrap.
                        if expected_is_carrier
                            && !self.is_constructor_matching_carrier(expr, &expected)
                        {
                            last_type = self.infer_statement(stmt)?;
                            continue;
                        }
                        let expr_type = self.check_against(expr, &expected)?;
                        self.record_implicit_return_type(expr_type.clone());
                        last_type = expr_type;
                        continue;
                    }
                    // Tail `if`/`else` whose every branch tail is itself a
                    // carrier-matching constructor (`if c { Ok(x*2) } else {
                    // Err("…") }`): thread the expected carrier into each branch
                    // tail so the constructor-payload adoption fires per branch.
                    // Gated on ALL branches being constructors so a mixed/bare
                    // branch (`if c { Ok(1) } else { 2 }`) still keeps the
                    // per-branch implicit-wrap path (plain inference).
                    Statement::If(if_stmt, _)
                        if expected_is_carrier
                            && self.if_branches_all_carrier_constructors(if_stmt, &expected) =>
                    {
                        last_type = self.check_if_stmt_against_carrier(if_stmt, &expected)?;
                        self.record_implicit_return_type(last_type.clone());
                        continue;
                    }
                    _ => {}
                }
            }
            last_type = self.infer_statement(stmt)?;
        }
        Ok(last_type)
    }

    /// Whether `expr` is an `Ok`/`Err`/`Some` constructor whose expected payload
    /// type can be extracted from the `expected` carrier — i.e. the
    /// bidirectional constructor-payload arm of `check_against` would fire. Used
    /// to gate Result/Option tail/return adoption to constructor expressions
    /// only (bare values keep implicit-wrap).
    ///
    /// User enum tuple constructors (`Tagged::N(5)`) parse as
    /// `Expr::QualifiedFunctionCall` (the grammar's `qualified_function_call_expr`
    /// alternative wins the ordered choice over `enum_constructor_expr` whenever
    /// a `(args)` payload is present), and that path does not yet propagate or
    /// even check payload types — out of scope for this Ok/Err/Some FP-regression
    /// fix.
    fn is_constructor_matching_carrier(&self, expr: &Expr, expected: &Type) -> bool {
        match expr {
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some") =>
            {
                // Only intercept when the argument is a bare numeric literal
                // that adopts the expected payload — a non-literal `Ok(x)` /
                // `Ok(x * 2)` is left to `infer_function_call` (its success var
                // links to the carrier's success type via the lenient
                // `Result<var> ~ Result<number>` unification, with no hard
                // per-operand constraint that would conflict with an int-pinning
                // guard).
                self.constructor_arg_adopts_literal(name, args, expected)
            }
            // A tail `if`/`else` that PARSES as an expression-valued
            // `Expr::Conditional` (`if c { Ok(42) } else { Err("…") }` in tail
            // position). Each branch is an `Expr::Block` (the `{ … }` body), so
            // the structural / adopts checks recurse through the block tail.
            // Routed through `check_against` (which propagates the expected
            // carrier into each branch and threads through the block to its tail
            // constructor) ONLY when BOTH branch tails are structurally carrier
            // constructors AND at least one branch's constructor argument adopts
            // a numeric literal — mirroring the `if_branches_all_carrier_
            // constructors` gate used for the `Statement::If` parse. A branch
            // whose constructor argument is a NON-literal (`Ok(x*2)`, `Err("…")`)
            // routes through the default `check_against` path inside the block
            // (plain inference + lenient `Result<var> ~ Result<number>`), so the
            // literal `then` branch adopts without forcing the non-literal one.
            Expr::Conditional {
                then_expr,
                else_expr: Some(else_expr),
                ..
            } if self.block_tail_is_carrier_constructor(then_expr, expected)
                && self.block_tail_is_carrier_constructor(else_expr, expected)
                && (self.block_tail_adopts_literal(then_expr, expected)
                    || self.block_tail_adopts_literal(else_expr, expected)) =>
            {
                true
            }
            _ => false,
        }
    }

    /// Whether `expr` is an `Expr::Block` whose tail expression item is
    /// STRUCTURALLY a carrier constructor (or nested carrier conditional) for
    /// `expected`. Used by the tail-`Conditional` gate to require both branches
    /// be constructors before threading. Recurses through nested blocks /
    /// conditionals.
    fn block_tail_is_carrier_constructor(&self, expr: &Expr, expected: &Type) -> bool {
        match expr {
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some") =>
            {
                self.constructor_payload_types_from_expected(name, args.len(), expected)
                    .is_some()
            }
            Expr::Conditional {
                then_expr,
                else_expr: Some(else_expr),
                ..
            } => {
                self.block_tail_is_carrier_constructor(then_expr, expected)
                    && self.block_tail_is_carrier_constructor(else_expr, expected)
            }
            Expr::Block(block, _) => match block.items.last() {
                Some(shape_ast::ast::BlockItem::Expression(tail)) => {
                    self.block_tail_is_carrier_constructor(tail, expected)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether `expr` is an `Expr::Block` whose tail constructor argument ADOPTS
    /// a numeric literal — i.e. threading the carrier into this branch would
    /// actually change the outcome. Recurses through nested blocks /
    /// conditionals.
    fn block_tail_adopts_literal(&self, expr: &Expr, expected: &Type) -> bool {
        match expr {
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some") =>
            {
                self.constructor_arg_adopts_literal(name, args, expected)
            }
            Expr::Conditional {
                then_expr,
                else_expr: Some(else_expr),
                ..
            } => {
                self.block_tail_adopts_literal(then_expr, expected)
                    || self.block_tail_adopts_literal(else_expr, expected)
            }
            Expr::Block(block, _) => match block.items.last() {
                Some(shape_ast::ast::BlockItem::Expression(tail)) => {
                    self.block_tail_adopts_literal(tail, expected)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether `expr` is STRUCTURALLY an `Ok`/`Err`/`Some` constructor that
    /// matches the `expected` carrier (regardless of whether its argument
    /// adopts a literal). Used by the conditional-tail gate to require all
    /// branches be constructors so threading each through `check_against` is
    /// safe (a non-adopting branch routes through the default path inside
    /// `check_body_tail_against_carrier`).
    fn is_carrier_constructor_struct(&self, expr: &Expr, expected: &Type) -> bool {
        match expr {
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some") =>
            {
                self.constructor_payload_types_from_expected(name, args.len(), expected)
                    .is_some()
            }
            _ => false,
        }
    }

    /// Whether the tail statement of a branch body is structurally a carrier
    /// constructor (or a nested all-constructor `if`).
    fn body_tail_is_carrier_constructor(&self, stmts: &[Statement], expected: &Type) -> bool {
        match stmts.last() {
            Some(Statement::Expression(expr, _)) => {
                self.is_carrier_constructor_struct(expr, expected)
            }
            Some(Statement::If(if_stmt, _)) => {
                self.if_branches_all_carrier_constructors(if_stmt, expected)
            }
            _ => false,
        }
    }

    /// Whether the tail statement of a branch body has a constructor argument
    /// that ADOPTS a literal — i.e. threading the carrier into this branch
    /// would actually change the outcome. Used to require at least one branch
    /// benefit before threading a tail `if` (so a `Result<int>` if/else of two
    /// identity constructors keeps plain inference unchanged).
    fn body_tail_adopts_literal(&self, stmts: &[Statement], expected: &Type) -> bool {
        match stmts.last() {
            Some(Statement::Expression(expr, _)) => {
                self.is_constructor_matching_carrier(expr, expected)
            }
            Some(Statement::If(if_stmt, _)) => {
                if let Some(else_body) = &if_stmt.else_body {
                    self.body_tail_adopts_literal(&if_stmt.then_body, expected)
                        || self.body_tail_adopts_literal(else_body, expected)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Whether to thread the expected carrier into a tail `if`/`else`: BOTH
    /// branches are structurally carrier constructors (so `check_against`
    /// per-branch is safe), an `else` branch is present (a one-armed `if` keeps
    /// plain inference for the implicit fall-through), AND at least one branch's
    /// constructor argument adopts a numeric literal (so the threading is not a
    /// no-op that would needlessly re-route identity constructors).
    fn if_branches_all_carrier_constructors(
        &self,
        if_stmt: &shape_ast::ast::IfStatement,
        expected: &Type,
    ) -> bool {
        let Some(else_body) = &if_stmt.else_body else {
            return false;
        };
        let both_constructors = self.body_tail_is_carrier_constructor(&if_stmt.then_body, expected)
            && self.body_tail_is_carrier_constructor(else_body, expected);
        let any_adopts = self.body_tail_adopts_literal(&if_stmt.then_body, expected)
            || self.body_tail_adopts_literal(else_body, expected);
        both_constructors && any_adopts
    }

    /// Type-check a tail `if`/`else` against an expected Result/Option carrier,
    /// threading the carrier into each branch's tail (mirrors the
    /// `Statement::If` handler in `infer_statement`, including flow narrowing
    /// and conditional scopes, but uses `check_body_tail_against_carrier` for
    /// the branch bodies so the constructor-payload adoption fires per branch).
    /// Caller guarantees both branches are carrier-matching constructors.
    fn check_if_stmt_against_carrier(
        &mut self,
        if_stmt: &shape_ast::ast::IfStatement,
        expected: &Type,
    ) -> TypeResult<Type> {
        self.infer_expr(&if_stmt.condition)?;
        let narrowings = self.extract_narrowings(&if_stmt.condition);

        self.env.enter_conditional();
        self.env.push_scope();
        for (var_name, narrowed_type) in &narrowings {
            self.env
                .define(var_name, TypeScheme::mono(narrowed_type.clone()));
        }
        let then_type = self.check_body_tail_against_carrier(&if_stmt.then_body, expected)?;
        self.env.pop_scope();
        self.env.exit_conditional();

        // Guaranteed Some by the gate (`if_branches_all_carrier_constructors`
        // requires an else branch), but match defensively.
        if let Some(else_body) = &if_stmt.else_body {
            let inverse_narrowings = self.extract_inverse_narrowings(&if_stmt.condition);
            self.env.enter_conditional();
            self.env.push_scope();
            for (var_name, narrowed_type) in &inverse_narrowings {
                self.env
                    .define(var_name, TypeScheme::mono(narrowed_type.clone()));
            }
            let else_type = self.check_body_tail_against_carrier(else_body, expected)?;
            self.env.pop_scope();
            self.env.exit_conditional();
            self.constraints.push((then_type.clone(), else_type));
        }
        Ok(then_type)
    }

    /// Infer a branch body, threading the expected carrier into the tail
    /// statement only (non-tail statements keep plain inference). The tail is a
    /// carrier-matching constructor (→ `check_against` for payload adoption) or
    /// a nested carrier-`if` (→ recurse). The caller's gate guarantees the tail
    /// is one of those shapes.
    fn check_body_tail_against_carrier(
        &mut self,
        stmts: &[Statement],
        expected: &Type,
    ) -> TypeResult<Type> {
        let mut last_type = BuiltinTypes::void();
        let n = stmts.len();
        for (idx, stmt) in stmts.iter().enumerate() {
            let is_tail = idx + 1 == n;
            if is_tail {
                match stmt {
                    Statement::Expression(expr, _)
                        if self.is_constructor_matching_carrier(expr, expected) =>
                    {
                        last_type = self.check_against(expr, expected)?;
                        continue;
                    }
                    Statement::If(if_stmt, _)
                        if self.if_branches_all_carrier_constructors(if_stmt, expected) =>
                    {
                        last_type = self.check_if_stmt_against_carrier(if_stmt, expected)?;
                        continue;
                    }
                    _ => {}
                }
            }
            last_type = self.infer_statement(stmt)?;
        }
        Ok(last_type)
    }

    /// Numeric-conversion §4 literal adoption for an explicit `return <expr>`:
    /// when the enclosing fn declares a numeric return type and `expr` is a bare
    /// int literal that losslessly fits it, return the declared numeric type so
    /// the recorded return is `number` (not `int`). Otherwise the originally
    /// inferred type is returned unchanged.
    fn adopt_return_literal(&self, expr: &Expr, inferred: Type) -> Type {
        if let Some(Some(expected)) = self.expected_return_types.last() {
            if let Some(adopted) = Self::adopt_int_literal_in_context(expr, expected) {
                return adopted;
            }
        }
        inferred
    }

    /// Infer return type for a callable body.
    ///
    /// - If the body contains explicit `return` statements, aggregate all
    ///   returned types (including from nested control-flow) into a single type.
    /// - If no explicit `return` exists, use the final statement type to support
    ///   expression-style bodies.
    pub(crate) fn infer_callable_return_type(
        &mut self,
        stmts: &[Statement],
        allow_unresolved_generic_args: bool,
    ) -> TypeResult<Type> {
        self.push_return_scope();
        self.push_implicit_return_scope();
        let body_result = self.infer_statements_with_return_adoption(stmts);
        let explicit_returns = self.pop_return_scope();
        let implicit_returns = self.pop_implicit_return_scope();
        let body_type = body_result?;
        let implicit_candidates: Vec<Type> = implicit_returns
            .into_iter()
            .filter(|ty| !self.is_void_type(ty))
            .collect();

        if explicit_returns.is_empty() {
            if implicit_candidates.is_empty() {
                Ok(body_type)
            } else {
                if allow_unresolved_generic_args {
                    self.combine_return_types_allow_unresolved(&implicit_candidates)
                } else {
                    self.combine_return_types(&implicit_candidates)
                }
            }
        } else {
            // A body may return through both explicit statements and a fallthrough
            // tail expression, e.g. `if bad { return Err(e) }; Ok(value)`.
            // Both paths are callable returns and must share one carrier.
            let mut return_candidates = explicit_returns;
            return_candidates.extend(implicit_candidates);

            if self.all_types_equal(&return_candidates) {
                Ok(return_candidates[0].clone())
            } else if let Some(base_var) = return_candidates.iter().find_map(|ty| match ty {
                Type::Variable(var) => Some(var.clone()),
                _ => None,
            }) {
                // Preserve precision for mixed returns like:
                //   return c        // c: type variable resolved from call-sites
                //   return "hi"     // concrete
                // by materializing the union after call-site widening.
                let additional_members = return_candidates
                    .iter()
                    .filter(|ty| !matches!(ty, Type::Variable(var) if *var == base_var))
                    .cloned()
                    .collect::<Vec<_>>();
                if additional_members.is_empty() {
                    Ok(Type::Variable(base_var))
                } else {
                    self.record_pending_return_union(base_var.clone(), additional_members);
                    Ok(Type::Variable(base_var))
                }
            } else {
                if allow_unresolved_generic_args {
                    self.combine_return_types_allow_unresolved(&return_candidates)
                } else {
                    self.combine_return_types(&return_candidates)
                }
            }
        }
    }

    fn is_constructor_like_call_name(name: &str) -> bool {
        matches!(name, "Ok" | "Err" | "Some" | "None")
            || name.chars().next().is_some_and(char::is_uppercase)
    }

    fn expression_statement_records_implicit_return(expr: &Expr) -> bool {
        match expr {
            Expr::FunctionCall { name, .. } => Self::is_constructor_like_call_name(name),
            Expr::QualifiedFunctionCall { function, .. } => {
                Self::is_constructor_like_call_name(function)
            }
            Expr::MethodCall { .. } => false,
            Expr::EnumConstructor { .. } => true,
            _ => true,
        }
    }

    /// Infer type of a single statement
    pub(crate) fn infer_statement(&mut self, stmt: &Statement) -> TypeResult<Type> {
        match stmt {
            Statement::Return(expr_opt, _) => {
                let return_type = if let Some(expr) = expr_opt {
                    // Numeric-conversion §4 literal adoption (return context):
                    //   - a bare int literal `return <lit>` adopts the enclosing
                    //     fn's declared numeric return type when it losslessly
                    //     fits (`fn f() -> number { return 42 }` returns
                    //     `number`, not `int`); and
                    //   - an `Ok`/`Err`/`Some` (or user-enum) constructor
                    //     `return Ok(42)` against a declared `Result<number>`
                    //     propagates the expected payload to the constructor's
                    //     argument so the literal adopts `number`
                    //     (constructor-payload-vs-expected path), mirroring the
                    //     tail-expression handling.
                    let carrier_expected = match self.expected_return_types.last() {
                        Some(Some(ty))
                            if (self.is_result_type(ty) || self.is_option_type(ty))
                                && self.is_constructor_matching_carrier(expr, ty) =>
                        {
                            Some(ty.clone())
                        }
                        _ => None,
                    };
                    if let Some(expected) = carrier_expected {
                        self.check_against(expr, &expected)?
                    } else {
                        let inferred = self.infer_expr(expr)?;
                        self.adopt_return_literal(expr, inferred)
                    }
                } else {
                    BuiltinTypes::void()
                };
                self.record_return_type(return_type.clone());
                Ok(return_type)
            }
            Statement::VariableDecl(decl, _) => {
                self.infer_variable_decl(decl)?;
                Ok(BuiltinTypes::void())
            }
            Statement::Assignment(assign, span) => {
                self.infer_assignment(assign, *span)?;
                Ok(BuiltinTypes::void())
            }
            Statement::Expression(expr, _) => {
                let expr_type = self.infer_expr(expr)?;
                // Fluent mutator method statements are conventionally
                // discarded (`m.set(k, v)`, `arr.push(x)`). They still get
                // inferred above for constraints, but they do not become the
                // enclosing function's body type. Value-producing methods
                // (`map`, `filter`, `sum`, callable `__call__`, etc.) stay
                // eligible as expression-style returns.
                if matches!(expr, Expr::MethodCall { method, .. } if Self::method_statement_discards_value(method))
                {
                    return Ok(BuiltinTypes::void());
                }
                // Record the expression's type as an implicit-return candidate.
                // Recording ordinary method-call statements previously unioned
                // the receiver's type (e.g.
                // `HashMap<string,int>` from `m.set(...)`) into the enclosing
                // fn's implicit return, producing a spurious
                // `() -> HashMap<…> | int` constraint mis-solve.
                //
                // Value-producing statements that ARE legitimate implicit-
                // return contributors (constructor calls `Ok(1)` / `Err(e)`,
                // bare values) keep recording — Shape collects these across
                // multiple statements for `Result`/union return inference
                // (`fn f() { Ok(1) \n Err("e") }` ⇒ `Result<int, string>`).
                if Self::expression_statement_records_implicit_return(expr) {
                    self.record_implicit_return_type(expr_type.clone());
                }
                Ok(expr_type)
            }
            Statement::If(if_stmt, _) => {
                self.infer_expr(&if_stmt.condition)?;

                // Extract flow-sensitive narrowing info from the condition
                let narrowings = self.extract_narrowings(&if_stmt.condition);

                // Enter conditional context for field evolution tracking
                self.env.enter_conditional();
                self.env.push_scope();
                // Push narrowed types for then-branch (e.g. x != null → x: T)
                for (var_name, narrowed_type) in &narrowings {
                    self.env
                        .define(var_name, TypeScheme::mono(narrowed_type.clone()));
                }
                // A branch body that DIVERGES (its last statement is — or is
                // dominated by — a `return`/`break`/`continue`) is the
                // NEVER/bottom type: it produces no value and must be EXCLUDED
                // from the branch-type unification. We still INFER the branch
                // (so a `return Err(...)`'s value is checked against the fn
                // return type), but we do NOT unify a diverging branch's type
                // against the other branch, and the diverging branch does not
                // become the if-statement's type. This is the ROOT fix for
                // `if cond { acc = v } else { return Err(...) }`: the else body
                // ends in `return Err(...)` (typed as the fn's `Result<…>`),
                // which previously unified against the void then-branch and
                // wrongly rejected.
                let then_diverges = Self::body_diverges(&if_stmt.then_body);
                let then_type = self.infer_statements(&if_stmt.then_body)?;
                self.env.pop_scope();
                self.env.exit_conditional();

                if let Some(else_body) = &if_stmt.else_body {
                    // Compute inverse narrowings for else-branch
                    let inverse_narrowings = self.extract_inverse_narrowings(&if_stmt.condition);
                    self.env.enter_conditional();
                    self.env.push_scope();
                    for (var_name, narrowed_type) in &inverse_narrowings {
                        self.env
                            .define(var_name, TypeScheme::mono(narrowed_type.clone()));
                    }
                    let else_diverges = Self::body_diverges(else_body);
                    let else_type = self.infer_statements(else_body)?;
                    self.env.pop_scope();
                    self.env.exit_conditional();

                    match (then_diverges, else_diverges) {
                        // Both diverge → the whole if/else is Never.
                        (true, true) => Ok(Type::Concrete(TypeAnnotation::Never)),
                        // Only else diverges → if-statement type is the then body.
                        (false, true) => Ok(then_type),
                        // Only then diverges → if-statement type is the else body.
                        (true, false) => Ok(else_type),
                        // Neither diverges → ordinary branch-type unification.
                        (false, false) => {
                            self.constraints.push((then_type.clone(), else_type));
                            Ok(then_type)
                        }
                    }
                } else if then_diverges {
                    // then-only `if` whose body diverges: still falls through
                    // when the condition is false, so the statement is void.
                    Ok(BuiltinTypes::void())
                } else {
                    Ok(then_type)
                }
            }
            Statement::For(for_loop, _) => {
                self.env.push_scope();

                // Handle different for loop types
                match &for_loop.init {
                    shape_ast::ast::ForInit::ForIn { pattern, iter } => {
                        let iter_type = self.infer_expr(iter)?;

                        // Infer element type from iterator
                        let element_type = self.infer_iterator_element_type(&iter_type)?;
                        // ROOT-1 (strict-flip, 2026-06-18): an OBJECT-destructuring
                        // for-in (`for {x, y} in [P{..}]`) must type each binder
                        // from the element struct's declared FIELD annotation, not
                        // from the whole element struct, else `x + y` rejects with
                        // "P does not implement Numeric". (Mirror of the
                        // `Expr::For` arm in `inference/expressions.rs`.) A field
                        // whose type is unresolvable, and every non-object pattern,
                        // falls back to the element type (parity preserved).
                        self.bind_for_in_destructure_pattern(pattern, &element_type);
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.infer_statement(init)?;
                        let cond_type = self.infer_expr(condition)?;
                        self.constraints.push((cond_type, BuiltinTypes::boolean()));
                        self.infer_expr(update)?;
                    }
                }

                // Enter loop context for field evolution tracking
                self.env.enter_loop();
                self.infer_statements(&for_loop.body)?;
                self.env.exit_loop();
                self.env.pop_scope();

                Ok(BuiltinTypes::void())
            }
            Statement::While(while_loop, _) => {
                self.infer_expr(&while_loop.condition)?;
                // Enter loop context for field evolution tracking
                self.env.enter_loop();
                self.infer_statements(&while_loop.body)?;
                self.env.exit_loop();
                Ok(BuiltinTypes::void())
            }
            _ => Ok(BuiltinTypes::void()),
        }
    }

    fn method_statement_discards_value(method: &str) -> bool {
        matches!(method, "push" | "set" | "delete" | "pushBack" | "pushFront")
    }

    /// Extract narrowing info from a condition expression.
    /// For `x != null`, returns `[(x, T)]` where the original type of x is `T?`.
    fn extract_narrowings(&mut self, condition: &Expr) -> Vec<(String, Type)> {
        match condition {
            // x != null  or  x != undefined  →  narrow x from T? to T
            Expr::BinaryOp {
                left,
                op: BinaryOp::NotEqual,
                right,
                ..
            } => {
                if Self::is_null_literal(right) {
                    self.try_null_narrowing(left)
                } else if Self::is_null_literal(left) {
                    self.try_null_narrowing(right)
                } else {
                    vec![]
                }
            }
            // x == null  →  no narrowing in then-branch (narrowing in else-branch)
            _ => vec![],
        }
    }

    /// Extract inverse narrowings for else-branch.
    /// For `x == null`, returns `[(x, T)]` (else means x is not null).
    /// For `x != null`, no narrowing in else-branch.
    fn extract_inverse_narrowings(&mut self, condition: &Expr) -> Vec<(String, Type)> {
        match condition {
            // x == null  →  in the else-branch, x is not null → narrow T? to T
            Expr::BinaryOp {
                left,
                op: BinaryOp::Equal,
                right,
                ..
            } => {
                if Self::is_null_literal(right) {
                    self.try_null_narrowing(left)
                } else if Self::is_null_literal(left) {
                    self.try_null_narrowing(right)
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    /// Check if an expression is a null/none literal.
    fn is_null_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::None, _) => true,
            Expr::Identifier(name, _) => name == "null" || name == "undefined" || name == "none",
            _ => false,
        }
    }

    /// Try to narrow a variable from T? to T.
    /// Returns narrowing if the expression is a variable with an Optional type.
    fn try_null_narrowing(&mut self, expr: &Expr) -> Vec<(String, Type)> {
        if let Expr::Identifier(name, _) = expr {
            let scheme = self.env.lookup(name).cloned();
            if let Some(scheme) = scheme {
                let ty = scheme.instantiate(&mut self.type_var_gen);
                if let Some(inner) = Self::unwrap_optional_type(&ty) {
                    return vec![(name.clone(), inner)];
                }
            }
        }
        vec![]
    }

    /// Unwrap T? / Option<T> to T.
    fn unwrap_optional_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Option" && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            Type::Generic { base, args } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    if name == "Option" && args.len() == 1 {
                        return Some(args[0].clone());
                    }
                }
                None
            }
            _ => None,
        }
    }
}
