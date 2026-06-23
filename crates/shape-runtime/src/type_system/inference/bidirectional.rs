//! Bidirectional Type Checking
//!
//! Implements bidirectional type checking for improved type inference,
//! especially for closure expressions passed to higher-order functions
//! where the expected parameter types can be propagated inward.
//!
//! ## Check Modes
//!
//! - **`Infer`** -- No expected type; purely synthesise from the expression.
//! - **`Check(Type)`** -- Hard constraint: the expression *must* have this
//!   type. Emitted for explicitly annotated bindings and return positions.
//!   A mismatch is a type error.
//! - **`Synth(Type)`** -- Soft hint: the expression is *expected* to have
//!   this type but may refine it. Used when propagating closure parameter
//!   types inferred from generic method signatures (e.g. the element type
//!   `T` from `Vec<T>.map(fn(T) -> U) -> Vec<U>`).
//!
//! ## Flow
//!
//! `check_expr` dispatches on the mode:
//! - `Infer` falls through to `infer_expr` (pure synthesis).
//! - `Check` calls `check_against`, which infers the expression and then
//!   emits an equality constraint between inferred and expected types.
//! - `Synth` calls `synthesize_with_hint`, which infers the expression,
//!   emits the constraint, and returns the inferred type (not the hint)
//!   so downstream inference stays precise.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Expr, FunctionParameter, ObjectEntry, TypeAnnotation};

/// Mode for bidirectional type checking
#[derive(Debug, Clone)]
pub enum CheckMode {
    /// Infer the type without any expectation
    Infer,
    /// Check the expression against an expected type (hard constraint)
    Check(Type),
    /// Synthesize with a hint type (soft constraint)
    Synth(Type),
}

impl CheckMode {
    /// Get the expected type if in Check or Synth mode
    pub fn expected(&self) -> Option<&Type> {
        match self {
            CheckMode::Infer => None,
            CheckMode::Check(ty) | CheckMode::Synth(ty) => Some(ty),
        }
    }

    /// Check if this is a hard constraint
    pub fn is_hard_constraint(&self) -> bool {
        matches!(self, CheckMode::Check(_))
    }
}

impl TypeInferenceEngine {
    /// Check an expression with a given mode
    ///
    /// This is the main entry point for bidirectional type checking.
    pub fn check_expr(&mut self, expr: &Expr, mode: CheckMode) -> TypeResult<Type> {
        match mode {
            CheckMode::Infer => self.infer_expr(expr),
            CheckMode::Check(expected) => self.check_against(expr, &expected),
            CheckMode::Synth(hint) => self.synth_with_hint(expr, &hint),
        }
    }

    /// Check an expression against an expected type
    ///
    /// The expected type guides inference and provides better error messages.
    pub fn check_against(&mut self, expr: &Expr, expected: &Type) -> TypeResult<Type> {
        match expr {
            // Function expression: use expected function type for parameter inference
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                span: _span,
            } => self.check_function_expr_against(params, return_type.as_ref(), body, expected),

            // Array: propagate element type to elements
            Expr::Array(elements, _) => {
                // U1: the expected array type may arrive in ANY encoding — the
                // canonical `Generic{Array, [elem]}` (now produced by every array
                // literal + `BuiltinTypes::array`), the annotation
                // `Concrete(Generic{name:"Array"/"Vec"})`, or the legacy
                // `Concrete(Array(..))`. Canonicalize and extract the single
                // element type so it propagates into the literal's elements
                // (enabling per-element numeric literal width-adoption, e.g.
                // `let a: Array<i32> = [1,2,3]`) regardless of which encoding the
                // annotation produced.
                let canon_expected = expected.canonicalize();
                if let Type::Generic { base, args } = &canon_expected {
                    let is_array_base = matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(tp))
                            if { let n = tp.to_string(); n == "Array" || n == "Vec" }
                    );
                    if is_array_base && args.len() == 1 {
                        return self.check_array_against(elements, &args[0]);
                    }
                }
                match expected {
                    Type::Concrete(TypeAnnotation::Array(elem_ty)) => {
                        self.check_array_against(elements, &Type::Concrete(*elem_ty.clone()))
                    }
                    // Tuple type (book `fundamentals/variables` §Tuple Types):
                    // `[T1, T2, ...]` is the bracket-syntax tuple type, and a
                    // bracket literal `[v1, v2, ...]` is its value form. There is
                    // no `(a, b)` paren tuple literal — the literal IS an
                    // `Expr::Array`. Check each element AGAINST its positional
                    // element type (so heterogeneous tuples like `[int, string]`
                    // type-check per position) and verify the arity. No coercion:
                    // each element must satisfy its declared position type.
                    Type::Concrete(TypeAnnotation::Tuple(elem_types)) => {
                        self.check_tuple_against(elements, elem_types)
                    }
                    _ => {
                        // Expected isn't an array/tuple type, infer and unify
                        let inferred = self.infer_expr(expr)?;
                        self.constraints.push((inferred.clone(), expected.clone()));
                        Ok(inferred)
                    }
                }
            }

            // Object: propagate field types
            Expr::Object(entries, _) => {
                if let Type::Concrete(TypeAnnotation::Object(expected_fields)) = expected {
                    self.check_object_against(entries, expected_fields)
                } else {
                    let inferred = self.infer_expr(expr)?;
                    self.constraints.push((inferred.clone(), expected.clone()));
                    Ok(inferred)
                }
            }

            // Conditional: propagate expected to both branches
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                let cond_type = self.infer_expr(condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                // A DIVERGING branch (`return`/`break`/`continue`) is NEVER and
                // is EXCLUDED from the branch-type unification — same rule as
                // the `Expr::Conditional` / `Statement::If` inference paths.
                let then_diverges = Self::expr_diverges(then_expr);
                let then_type = self.check_against(then_expr, expected)?;

                if let Some(else_e) = else_expr {
                    let else_diverges = Self::expr_diverges(else_e);
                    let else_type = self.check_against(else_e, expected)?;
                    match (then_diverges, else_diverges) {
                        (true, true) => Ok(Type::Concrete(TypeAnnotation::Never)),
                        (false, true) => Ok(then_type),
                        (true, false) => Ok(else_type),
                        (false, false) => {
                            self.constraints.push((then_type.clone(), else_type));
                            Ok(then_type)
                        }
                    }
                } else if then_diverges {
                    Ok(BuiltinTypes::void())
                } else {
                    Ok(then_type)
                }
            }

            // Match: propagate expected to arms
            Expr::Match(match_expr, _) => {
                let raw_scrutinee_type = self.infer_expr(&match_expr.scrutinee)?;
                // Zonk the scrutinee through the unifier's substitution store
                // BEFORE binding pattern vars, then thread it into
                // `bind_pattern_vars_typed`. This mirrors the `infer_expr` Match
                // path (expressions.rs:1291-1302). The bidirectional / tail
                // (return-position) path previously DISCARDED the scrutinee and
                // called the scrutinee-less `bind_pattern_vars`, which routed to
                // `check_constructor_pattern_ownership(None, variant)` — that
                // cannot prove a foreign constructor pattern (e.g. `Some(n)`
                // against a `Result<Point,…>` scrutinee) is non-enum, so it
                // surfaced-and-stopped, accepted the foreign pattern, and bound
                // the payload to a fresh unknown. The unknown then flowed past
                // FIX-B and got coerced to the concrete declared return type at
                // the boundary, producing a heap-pointer reinterpret. Threading
                // the real scrutinee gives the ownership check the type it needs
                // to reject the foreign pattern.
                let scrutinee_type = self.unifier.apply_substitutions(&raw_scrutinee_type);

                let mut arm_types = Vec::new();
                let mut any_arm = false;
                let mut all_diverge = true;

                for arm in &match_expr.arms {
                    any_arm = true;
                    self.env.push_scope();
                    self.bind_pattern_vars_typed(&arm.pattern, Some(&scrutinee_type))?;

                    // A DIVERGING arm (`return`/`break`/`continue`) is NEVER and
                    // is EXCLUDED from the arm-type unification (still inferred
                    // so its inner constraints are recorded).
                    let arm_diverges = Self::expr_diverges(&arm.body);
                    let arm_type = self.check_against(&arm.body, expected)?;
                    if arm_diverges {
                        self.env.pop_scope();
                        continue;
                    }
                    all_diverge = false;
                    arm_types.push(arm_type);

                    self.env.pop_scope();
                }

                if any_arm && all_diverge {
                    return Ok(Type::Concrete(TypeAnnotation::Never));
                }

                // All non-diverging arms should have the same type (the expected type)
                if !arm_types.is_empty() {
                    let first = arm_types[0].clone();
                    for ty in &arm_types[1..] {
                        self.constraints.push((first.clone(), ty.clone()));
                    }
                    Ok(first)
                } else {
                    Ok(expected.clone())
                }
            }

            // Block: the branch bodies of a tail `if`/`else` parse as
            // `Expr::Block`s, so the expected carrier must thread THROUGH the
            // block to its tail expression item for the constructor-payload
            // adoption to reach `Ok(x*2)` / `Err("…")`. Mirrors the `infer_expr`
            // block walk (same scope + per-item inference, so callsites and
            // bindings are still recorded), but routes the FINAL expression item
            // through `check_against(expected)` instead of plain inference. A
            // non-expression tail (a block ending in a statement / decl) keeps
            // the inferred type and is unified with `expected` like the default
            // arm.
            Expr::Block(block, block_span) => {
                self.env.push_scope();
                let mut last_type = BuiltinTypes::void();
                let n = block.items.len();
                for (idx, item) in block.items.iter().enumerate() {
                    let is_tail = idx + 1 == n;
                    last_type = match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            self.infer_variable_decl(decl)?;
                            BuiltinTypes::void()
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            self.infer_assignment(assign, *block_span)?;
                            BuiltinTypes::void()
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => self.infer_statement(stmt)?,
                        shape_ast::ast::BlockItem::Expression(expr) if is_tail => {
                            self.check_against(expr, expected)?
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => self.infer_expr(expr)?,
                    };
                }
                self.env.pop_scope();
                // A block whose tail was a non-expression item never reached
                // `check_against`, so its inferred type still has to be unified
                // with `expected` (the default-arm contract); a block whose tail
                // WAS routed through `check_against` already returns `expected`.
                if !matches!(
                    block.items.last(),
                    Some(shape_ast::ast::BlockItem::Expression(_))
                ) {
                    self.constraints.push((last_type.clone(), expected.clone()));
                }
                Ok(last_type)
            }

            // Numeric-conversion LITERAL ADOPTION through an ENUM-CONSTRUCTOR
            // payload (spec §4, constructor-payload-vs-expected path). When an
            // `Ok`/`Err`/`Some` constructor (parsed as a `FunctionCall`) whose
            // argument is a bare numeric LITERAL is checked against an EXPECTED
            // `Result<T,E>` / `Option<T>` carrier, propagate the expected
            // variant-payload type to the constructor's argument so the literal
            // adopts the expected numeric type — exactly the adoption the direct
            // contexts (let-annotation, comparison, struct-field, match-arm)
            // already get. `fn f() -> Result<number> { Ok(42) }` then accepts
            // (42 adopts `number`).
            //
            // GATED on the argument being a bare numeric LITERAL that ACTUALLY
            // adopts the expected payload (`constructor_arg_adopts_literal`). A
            // non-literal argument (`Ok(x)` / `Ok(x * 2)`) is LEFT to the
            // default `infer_function_call` path: it produces a `Result<var>`
            // whose success var is linked to the carrier's success type by the
            // lenient `Result<var> ~ Result<number>` unification (the var
            // resolves to `number` with no hard per-operand constraint), exactly
            // as the plain-tail `fn f(x) -> Result<number> { Ok(x * 2) }` case
            // already resolves on the baseline. Intercepting a non-literal here
            // with `check_against(arg, payload)` instead pushes a HARD
            // `arg ~ number` equality, which conflicts with an int-pinning guard
            // (`if x > 0 { … }` types `x` as `int` via the literal `0`) and
            // regressed previously-accepted programs — so the intercept stays
            // bounded to the bare-literal FP-regression class. A `number` VALUE /
            // non-literal `int` still does NOT widen.
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some")
                    && self.constructor_arg_adopts_literal(name, args, expected) =>
            {
                let payloads = self
                    .constructor_payload_types_from_expected(name, args.len(), expected)
                    .expect("constructor_arg_adopts_literal proved payloads exist");
                for (arg, payload) in args.iter().zip(payloads.iter()) {
                    self.check_against(arg, payload)?;
                }
                Ok(expected.clone())
            }

            // Numeric-conversion LITERAL ADOPTION (spec §4): a bare integer
            // literal in a concrete numeric context adopts that context type
            // when its value is losslessly representable in it. `let n: number =
            // 5` types `5` as the number literal `5.0`; `let x: u8 = 200` types
            // `200` as a u8 literal. NO rejecting constraint is pushed (the
            // literal IS the expected type). An OUT-OF-RANGE literal does NOT
            // adopt — it falls through to the default arm, where `(int, u8)`
            // fails the tightened §2 lattice and `let x: u8 = 300` correctly
            // compile-rejects (never a silent wrap).
            Expr::Literal(..) if Self::adopt_int_literal_in_context(expr, expected).is_some() => {
                Ok(expected.clone())
            }

            // Default: infer and constrain to expected
            _ => {
                let inferred = self.infer_expr(expr)?;
                self.constraints.push((inferred.clone(), expected.clone()));
                Ok(inferred)
            }
        }
    }

    /// Expected payload type(s) for an `Ok`/`Err`/`Some` constructor checked
    /// against an EXPECTED `Result<T,E>` / `Option<T>` carrier.
    ///
    /// - `Some` against `Option<T>` → `[T]`
    /// - `Ok` against `Result<T,E>` → `[T]`
    /// - `Err` against `Result<T,E>` → `[E]`
    ///
    /// Returns `None` (so the bidirectional arm falls through to plain
    /// inference) when the constructor doesn't match the expected carrier, the
    /// arity isn't 1, or the payload type can't be extracted. The returned
    /// payload type may be a bare type VARIABLE (an unresolved `Result<T,E>`
    /// from an annotation still being solved); `check_against` against a
    /// variable simply pushes the normal equality constraint, so literal
    /// adoption only fires when the expected payload is concrete-numeric.
    pub(crate) fn constructor_payload_types_from_expected(
        &self,
        name: &str,
        arity: usize,
        expected: &Type,
    ) -> Option<Vec<Type>> {
        if arity != 1 {
            return None;
        }
        match name {
            "Some" if self.is_option_type(expected) => self
                .result_or_option_success_type(expected)
                .map(|t| vec![t]),
            "Ok" if self.is_result_type(expected) => self
                .result_or_option_success_type(expected)
                .map(|t| vec![t]),
            "Err" if self.is_result_type(expected) => {
                self.result_error_type(expected).map(|e| vec![e])
            }
            _ => None,
        }
    }

    /// Whether the `Ok`/`Err`/`Some` constructor `name(args)` has an argument
    /// that is a bare numeric LITERAL which ADOPTS the expected variant-payload
    /// type extracted from `expected`. This is the precise gate for the
    /// constructor-payload-vs-expected intercept: it fires only when re-routing
    /// the argument through `check_against` would change the outcome (a literal
    /// adopting a numeric payload), so non-literal arguments (`Ok(x)`,
    /// `Ok(x * 2)`) and out-of-range / non-numeric literals fall through to the
    /// default `infer_function_call` path unchanged.
    pub(crate) fn constructor_arg_adopts_literal(
        &self,
        name: &str,
        args: &[Expr],
        expected: &Type,
    ) -> bool {
        let Some(payloads) =
            self.constructor_payload_types_from_expected(name, args.len(), expected)
        else {
            return false;
        };
        args.iter()
            .zip(payloads.iter())
            .any(|(arg, payload)| Self::adopt_int_literal_in_context(arg, payload).is_some())
    }

    /// ROOT-B: whether a bare int LITERAL payload of an `Ok`/`Err`/`Some`
    /// constructor should DEFER to its fresh payload type-variable instead of
    /// pinning that variable to `int`.
    ///
    /// `fn run() -> Result<number> { let v = Ok(7)?; Ok(v) }` and
    /// `fn check(n) -> Result<number> { Ok(0) }` (and the `let mut sum = 0;
    /// … Ok(sum)` accumulator class) construct an `Ok`/`Some` whose argument is
    /// a bare int literal at a site WITHOUT a bidirectional expected carrier —
    /// the `?`-strips-then-rewraps chain, a recursion base case, or an
    /// accumulator seed. Without deferral the polymorphic constructor's payload
    /// var `T` is pinned to `int` by the literal (`Ok(7) : Result<int>`), and
    /// the resulting `Result<int>` / `Option<int>` then conflicts with the
    /// function's `Result<number>` / `Option<number>` return carrier
    /// (`Result<int> !~ Result<number>`).
    ///
    /// This mirrors `adopt_int_literal_into_var` (the comparison-partner var
    /// case): a bare int literal has no committed numeric family, so deferring
    /// it to the still-unresolved payload var introduces NO value widening —
    /// the var resolves later (to `number` from the return carrier, or to `int`
    /// if nothing else constrains it). LITERALS ONLY: a non-literal `int`
    /// VALUE payload (`Ok(x)`, `Ok(x * 2)`) is left untouched, so an int-VALUE
    /// never silently becomes a `number` (§5 value-level invariant). Gated to
    /// the three success/error carriers, and only when the corresponding param
    /// is a bare, still-unresolved `Type::Variable` (the freshly-instantiated
    /// `T`/`E` payload var) — a concrete or annotated payload type keeps its
    /// normal `int`-pinning behavior.
    pub(crate) fn constructor_literal_payload_defers_to_var(
        name: &str,
        arg: &Expr,
        param: &Type,
    ) -> bool {
        if !matches!(name, "Ok" | "Err" | "Some") {
            return false;
        }
        // The param must be the freshly-instantiated payload var (unresolved).
        if !matches!(param, Type::Variable(_)) {
            return false;
        }
        // Reuse the literal-shape + value-fits gate (decimal accepts any integer
        // literal, so it is a stable proxy for "this expr is a bare adoptable
        // integer literal"). Non-literal / float / typed-int args do not defer.
        let decimal_probe = Type::Concrete(TypeAnnotation::Basic("decimal".to_string()));
        Self::adopt_int_literal_in_context(arg, &decimal_probe).is_some()
    }

    /// Synthesize type with a hint (soft constraint)
    ///
    /// The hint guides inference but doesn't force the type.
    fn synth_with_hint(&mut self, expr: &Expr, hint: &Type) -> TypeResult<Type> {
        // R3-subcase struct-array HOF (strict-flip, 2026-06-14): a closure
        // argument whose hint is a function type (`(User) -> bool` resolved
        // from the method's registered signature against the receiver's
        // element type) must bind its PARAMETERS from the hint BEFORE the body
        // is inferred — otherwise the body sees the closure param as a fresh
        // type var and a field access (`u.score`) resolves to `unknown`, which
        // the strict-typing emitter rejects ("Cannot infer types for binary
        // operation"). The plain `infer_expr` below infers the body with bare
        // fresh params and only unifies the WHOLE function type afterwards, far
        // too late for an in-body field-access / binary-op to type-check. Route
        // a function-expression closure with a function-typed hint through the
        // param-binding `check_function_expr_against` path (the same path the
        // hard `CheckMode::Check` already uses). This is the bidirectional
        // closure-param inference CLAUDE.md describes; it carries the receiver
        // element type (`User`) into the closure scope so `u.score` resolves to
        // the field's real type. Not broad-suppression: an unproven element
        // (non-function hint, or a hint whose param shape doesn't match) still
        // falls back to the soft-unify probe below — no fabrication, no default.
        if let Expr::FunctionExpr {
            params,
            return_type,
            body,
            ..
        } = expr
        {
            if matches!(
                hint,
                Type::Function { .. } | Type::Concrete(TypeAnnotation::Function { .. })
            ) {
                return self.check_function_expr_against(params, return_type.as_ref(), body, hint);
            }
        }

        let inferred = self.infer_expr(expr)?;

        // Try to unify with hint - if it fails, just return inferred
        // This is a "soft" constraint that helps but doesn't force.
        // U1: the single equality relation (probe-mode solve) replaces the
        // deleted read-only `Unifier::try_unify` here.
        if self.solver.probe_equal(&inferred, hint) {
            Ok(hint.clone())
        } else {
            Ok(inferred)
        }
    }

    /// Check a function expression against an expected function type
    fn check_function_expr_against(
        &mut self,
        params: &[FunctionParameter],
        return_type_ann: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
        expected: &Type,
    ) -> TypeResult<Type> {
        // Extract expected param types and return type from expected function type
        let (expected_params, expected_return) = match expected {
            Type::Concrete(TypeAnnotation::Function {
                params: expected_param_anns,
                returns,
            }) => {
                let param_types: Vec<Type> = expected_param_anns
                    .iter()
                    .map(|p| Type::Concrete(p.type_annotation.clone()))
                    .collect();
                let return_type = Type::Concrete(*returns.clone());
                (param_types, return_type)
            }
            Type::Function {
                params: fp,
                returns: fr,
            } => (fp.clone(), *fr.clone()),
            _ => {
                // Expected isn't a function type, fall back to regular inference
                return self.infer_function_expr(params, return_type_ann, body);
            }
        };

        // Enter a new scope for the function
        self.env.push_scope();
        self.push_fallible_scope();

        // Bind parameters with expected types (or declared/fresh if not enough info)
        for (i, param) in params.iter().enumerate() {
            let param_type = if i < expected_params.len() {
                expected_params[i].clone()
            } else if let Some(ann) = &param.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };

            // Define all identifiers from the pattern
            for name in param.get_identifiers() {
                self.env.define(&name, TypeScheme::mono(param_type.clone()));
            }
        }

        let inferred_result = self.infer_callable_return_type(body, true);
        let was_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        let constrained_expected_return =
            self.apply_fallibility_to_return_type(expected_return.clone(), was_fallible);
        if was_fallible && !self.is_result_type(&expected_return) {
            self.constraints
                .push((constrained_expected_return.clone(), expected_return.clone()));
        }

        // Constrain inferred callable return to expected return type.
        //
        // STRICT-FLIP (v0.3.3 map-output element-stamp NARROWING): the hard
        // constraint here is ONLY load-bearing when the expected closure return
        // is a bare `Type::Variable` — the `MethodParam` OUTPUT-element var of
        // `map` / `flatMap` / `select` (`fn(ReceiverParam(0)) -> MethodParam(0)`),
        // where the closure's return type literally BECOMES the result array's
        // element. There it MUST be exact so `Array<int> != Array<number>` holds
        // (the soundness the element-stamp established): `int` stays `int`,
        // `number` stays `number`, they do NOT unify (CLAUDE.md §Type-System).
        //
        // For an element-PRESERVING / terminal method the expected return is
        // CONCRETE and the closure result is NOT the output element — it is
        // discarded ordering data or unit:
        //   - `sort`'s comparator `(T,T) -> number`, `orderBy` / `sortBy`'s key
        //     `(T) -> number` — the numeric family of the key is not stored, so
        //     `|x| x` over `Array<int>` (key returns `int`) must not hard-reject
        //     against the registered `number` expected return; the result is the
        //     SelfType array (`Array<int>` — element PRESERVED, never `number`);
        //   - `forEach`'s `(T) -> void` — a unit closure body whose inferred and
        //     expected return are both `void` produced a degenerate `void ~ void`
        //     constraint the solver rejected;
        //   - `reduce`'s accumulator return is bound through the `MethodParam`
        //     value-position path, not this var arm.
        // Route the concrete case through the SOFT unify probe (same semantics as
        // `synth_with_hint`'s fallback) so closure PARAMS are still bound (already
        // done above) but the non-load-bearing return does not hard-fail the
        // solve. This restores the pre-R3 soft behavior for these methods WITHOUT
        // loosening the map/flatMap element-var arm. No coercion, no fabrication.
        if matches!(constrained_expected_return, Type::Variable(_)) {
            self.constraints.push((
                inferred_return_type.clone(),
                constrained_expected_return.clone(),
            ));
        }
        // U1: the concrete-return branch previously called the read-only
        // `Unifier::try_unify` and discarded the result — a pure no-op whose
        // only purpose was to NOT push a hard constraint. With `try_unify`
        // deleted, the branch is simply the absence of a pushed constraint;
        // closure PARAMS were already bound above. No coercion, no fabrication.

        // STRICT-FLIP (v0.3.3 map/collect OUTPUT element stamp): when the
        // expected return is a bare `MethodParam` var (the
        // `(ReceiverParam(0)) -> MethodParam(0)` shape registered for `map` /
        // `flatMap`), eagerly bind it to the closure's PROVEN inferred return
        // type. The deferred constraint above binds it eventually, but the
        // method-call RESULT type (`Vec<MethodParam(0)>`) is resolved at the
        // call site BEFORE the deferred solver runs — without the eager bind it
        // stays `Vec<freshvar>`, a FREE tyvar that later unifies with ANY
        // annotation (`let r = [1,2,3].map(|x| x*2); let n: number = r[0]`
        // wrongly ACCEPTED). Parity with `filter`'s `SelfType` element (concrete
        // because the receiver is concrete). Per ADR-006 §2.7.5
        // stamp-at-compile-time: the closure's inferred return type IS the proof
        // — `int` stays `int`, `number` stays `number`, they do NOT unify
        // (CLAUDE.md §Type-System-Rules). An un-inferable closure return leaves
        // the var unbound (still-unresolved → no bind), so a numeric annotation
        // on the result REJECTS rather than coerces — no fabrication.
        let returned_type = if let Type::Variable(ret_var) = &constrained_expected_return {
            let resolved = self.unifier.apply_substitutions(&inferred_return_type);
            if !self.type_contains_unresolved_vars(&resolved)
                && self.unifier.lookup(ret_var).is_none()
            {
                self.unifier.bind(ret_var.clone(), resolved.clone());
                resolved
            } else {
                constrained_expected_return.clone()
            }
        } else {
            constrained_expected_return.clone()
        };

        // Build the function type using Type::Function to preserve type variables
        let mut actual_param_types: Vec<Type> = Vec::with_capacity(params.len());
        for (i, p) in params.iter().enumerate() {
            let ty = if i < expected_params.len() {
                expected_params[i].clone()
            } else if let Some(ann) = &p.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };
            actual_param_types.push(ty);
        }

        Ok(Type::Function {
            params: actual_param_types,
            returns: Box::new(returned_type),
        })
    }

    /// Infer a function expression (fallback when no expected type)
    fn infer_function_expr(
        &mut self,
        params: &[FunctionParameter],
        return_type_ann: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
    ) -> TypeResult<Type> {
        self.env.push_scope();
        self.push_fallible_scope();

        let mut param_types = Vec::new();
        for param in params {
            let param_type = if let Some(ann) = &param.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };

            // Define all identifiers from the pattern
            for name in param.get_identifiers() {
                self.env.define(&name, TypeScheme::mono(param_type.clone()));
            }
            param_types.push(param_type);
        }

        let inferred_result = self.infer_callable_return_type(body, return_type_ann.is_some());
        let was_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        // If return type is annotated, constrain inferred type to annotation.
        // When the annotation is a Result/Option and the inferred body type is
        // a bare success value, constrain against the success type (Shape
        // implicitly Ok/Some-wraps the return value of a fallible/optional
        // function).
        let return_type = if let Some(ann) = return_type_ann {
            let annotated = Type::Concrete(ann.clone());
            self.push_return_constraint(inferred_return_type, annotated.clone());
            annotated
        } else {
            inferred_return_type
        };
        let return_type = self.apply_fallibility_to_return_type(return_type, was_fallible);

        // Build function type using Type::Function to preserve type variables
        Ok(Type::Function {
            params: param_types,
            returns: Box::new(return_type),
        })
    }

    /// Check array elements against expected element type
    fn check_array_against(&mut self, elements: &[Expr], elem_type: &Type) -> TypeResult<Type> {
        for elem in elements {
            self.check_against(elem, elem_type)?;
        }
        Ok(BuiltinTypes::array(elem_type.clone()))
    }

    /// Check a bracket literal `[v1, v2, ...]` against an expected bracket type
    /// `[T1, T2, ...]`.
    ///
    /// USER RULING 2026-06-17 — bracket `[T, T, ...]` is a fixed-length
    /// HOMOGENEOUS group, not a heterogeneous tuple. There is NO heterogeneous
    /// carrier in the runtime (homogeneous-only), so a bracket annotation whose
    /// element types are NOT a single homogeneous element type is a clean
    /// COMPILE-ERROR that points the user at a struct. The arity must also match
    /// exactly. Element types are homogeneous when they are all structurally
    /// equal (`[int, int]`) or all in the fixed-width lossless numeric lattice
    /// (`[int, number]` — the `int` literals losslessly adopt `number`). A
    /// non-numeric type mixed with any different type (`[int, string]`) has no
    /// homogeneous element carrier and is rejected here.
    fn check_tuple_against(
        &mut self,
        elements: &[Expr],
        elem_types: &[TypeAnnotation],
    ) -> TypeResult<Type> {
        if elements.len() != elem_types.len() {
            return Err(TypeError::TypeMismatch(
                format!("tuple of {} elements", elem_types.len()),
                format!("bracket literal with {} elements", elements.len()),
            ));
        }
        // Homogeneous-only enforcement (USER RULING 2026-06-17). Reject a
        // heterogeneous bracket annotation with a struct hint BEFORE per-element
        // checking, so the user sees the design-level guidance rather than a
        // downstream array-element-inference message at lowering.
        if !Self::bracket_elem_types_are_homogeneous(elem_types) {
            let rendered = TypeAnnotation::Tuple(elem_types.to_vec()).to_type_string();
            let struct_fields: Vec<String> = elem_types
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let name = (b'a' + (i as u8 % 26)) as char;
                    format!("{}: {}", name, t.to_type_string())
                })
                .collect();
            return Err(TypeError::ConstraintViolation(format!(
                "heterogeneous tuple `{rendered}` is not supported; \
                 bracket types `[T, T, ...]` are homogeneous-only. \
                 Use a struct instead: `type T {{ {} }}`",
                struct_fields.join(", ")
            )));
        }
        for (elem, ty) in elements.iter().zip(elem_types.iter()) {
            self.check_against(elem, &Type::Concrete(ty.clone()))?;
        }
        Ok(Type::Concrete(TypeAnnotation::Tuple(elem_types.to_vec())))
    }

    /// A bracket annotation `[T1, T2, ...]` is HOMOGENEOUS (USER RULING
    /// 2026-06-17, homogeneous-only) when every element type is the same
    /// element type. Two forms qualify:
    ///
    /// - all element annotations are structurally equal (`[int, int]`,
    ///   `[string, string]`), or
    /// - all element annotations are in the fixed-width lossless numeric lattice
    ///   (`int` / `number` / `i8` / `f32` / ...), so the differing-but-numeric
    ///   positions (`[int, number]`) collapse to a single numeric element type
    ///   by lossless adoption.
    ///
    /// `decimal` / `bigint` are arbitrary-precision heap numerics, NOT in the
    /// fixed-width lossless lattice — they only count as homogeneous via the
    /// structural-equality form, never via the all-numeric form.
    fn bracket_elem_types_are_homogeneous(elem_types: &[TypeAnnotation]) -> bool {
        let Some(first) = elem_types.first() else {
            // Empty `[]` annotation has no heterogeneity to reject.
            return true;
        };
        // Form 1: all structurally equal.
        if elem_types
            .iter()
            .all(|t| crate::type_system::unification::structural_equality::annotations_equal(first, t))
        {
            return true;
        }
        // Form 2: all in the fixed-width lossless numeric lattice.
        let lossless_numeric = |t: &TypeAnnotation| -> bool {
            let name = match t {
                TypeAnnotation::Basic(n) => Some(n.as_str()),
                TypeAnnotation::Reference(p) => Some(p.as_str()),
                _ => None,
            };
            name.map(|n| BuiltinTypes::canonical_numeric_runtime_name(n).is_some())
                .unwrap_or(false)
        };
        elem_types.iter().all(lossless_numeric)
    }

    /// Check object entries against expected field types
    fn check_object_against(
        &mut self,
        entries: &[ObjectEntry],
        expected_fields: &[shape_ast::ast::ObjectTypeField],
    ) -> TypeResult<Type> {
        let mut result_fields = Vec::new();

        for entry in entries {
            match entry {
                ObjectEntry::Field {
                    key,
                    value,
                    type_annotation: _type_annotation,
                } => {
                    // Find expected field type if available
                    let expected_field_type = expected_fields
                        .iter()
                        .find(|f| &f.name == key)
                        .map(|f| Type::Concrete(f.type_annotation.clone()));

                    let field_type = if let Some(expected) = expected_field_type {
                        self.check_against(value, &expected)?
                    } else {
                        self.infer_expr(value)?
                    };

                    result_fields.push(shape_ast::ast::ObjectTypeField {
                        name: key.clone(),
                        optional: false,
                        type_annotation: field_type
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                        annotations: vec![],
                    });
                }
                ObjectEntry::Spread(expr) => {
                    // Infer the type of the spread expression and merge its fields.
                    // Explicit fields declared later in the literal override spread fields.
                    let spread_type = self.infer_expr(expr)?;
                    let spread_fields = self.extract_object_fields(&spread_type);
                    for sf in spread_fields {
                        result_fields.push(sf);
                    }
                }
            }
        }

        // Deduplicate fields: later entries (explicit fields) override earlier ones (spread fields).
        // This matches JS/TS semantics: { ...obj, x: 1 } means x: 1 overrides obj.x.
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for field in result_fields.into_iter().rev() {
            if seen.insert(field.name.clone()) {
                deduped.push(field);
            }
        }
        deduped.reverse();

        Ok(Type::Concrete(TypeAnnotation::Object(deduped)))
    }

    /// Extract object-typed fields from a type for spread merging.
    ///
    /// Handles:
    /// - `Type::Concrete(TypeAnnotation::Object(fields))` -- inline object types
    /// - `Type::Concrete(TypeAnnotation::Reference(name))` -- named struct types via type alias
    ///   or struct_type_defs lookup
    fn extract_object_fields(&self, ty: &Type) -> Vec<shape_ast::ast::ObjectTypeField> {
        match ty {
            Type::Concrete(TypeAnnotation::Object(fields)) => fields.clone(),
            Type::Concrete(TypeAnnotation::Reference(name)) => {
                // Try struct_type_defs first (registered during hoisting)
                if let Some(struct_def) = self.struct_type_defs.get(name.as_str()) {
                    return struct_def
                        .fields
                        .iter()
                        .map(|f| shape_ast::ast::ObjectTypeField {
                            name: f.name.clone(),
                            optional: false,
                            type_annotation: f.type_annotation.clone(),
                            annotations: vec![],
                        })
                        .collect();
                }
                // Fall back to type alias lookup (struct types are stored as Object aliases)
                if let Some(alias) = self.env.lookup_type_alias(name) {
                    if let TypeAnnotation::Object(fields) = &alias.type_annotation {
                        return fields.clone();
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_mode_expected() {
        let mode = CheckMode::Infer;
        assert!(mode.expected().is_none());

        let mode = CheckMode::Check(BuiltinTypes::number());
        assert!(mode.expected().is_some());

        let mode = CheckMode::Synth(BuiltinTypes::string());
        assert!(mode.expected().is_some());
    }

    #[test]
    fn test_check_mode_is_hard_constraint() {
        assert!(!CheckMode::Infer.is_hard_constraint());
        assert!(CheckMode::Check(BuiltinTypes::number()).is_hard_constraint());
        assert!(!CheckMode::Synth(BuiltinTypes::number()).is_hard_constraint());
    }

    #[test]
    fn test_extract_object_fields_from_inline_object() {
        let engine = super::super::TypeInferenceEngine::new();
        let ty = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
            shape_ast::ast::ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("string".to_string()),
                annotations: vec![],
            },
        ]));
        let fields = engine.extract_object_fields(&ty);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[1].name, "y");
    }

    // USER RULING 2026-06-17 — bracket `[T, T, ...]` is HOMOGENEOUS-ONLY; a
    // heterogeneous bracket annotation (`[int, string]`) is a clean compile
    // error pointing the user at a struct.
    #[test]
    fn homogeneous_bracket_tuple_classification() {
        let basic = |n: &str| TypeAnnotation::Basic(n.to_string());
        // Structurally-equal element types: homogeneous.
        assert!(TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("int"),
            basic("int"),
        ]));
        assert!(TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("string"),
            basic("string"),
        ]));
        // Differing but all in the fixed-width lossless numeric lattice:
        // homogeneous by lossless adoption (`[int, number]`).
        assert!(TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("int"),
            basic("number"),
        ]));
        // Heterogeneous: a non-numeric type mixed with a different type.
        assert!(!TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("int"),
            basic("string"),
        ]));
        assert!(!TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("string"),
            basic("int"),
        ]));
        // `decimal` is arbitrary-precision, NOT in the lossless lattice — only
        // homogeneous via structural equality, never the all-numeric form.
        assert!(!TypeInferenceEngine::bracket_elem_types_are_homogeneous(&[
            basic("int"),
            basic("decimal"),
        ]));
    }

    #[test]
    fn check_tuple_against_rejects_heterogeneous_with_struct_hint() {
        let mut engine = super::super::TypeInferenceEngine::new();
        let elements = vec![
            Expr::Literal(shape_ast::ast::Literal::Int(7), Default::default()),
            Expr::Literal(
                shape_ast::ast::Literal::String("x".to_string()),
                Default::default(),
            ),
        ];
        let elem_types = vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ];
        let err = engine
            .check_tuple_against(&elements, &elem_types)
            .expect_err("heterogeneous `[int, string]` must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("heterogeneous tuple") && msg.contains("Use a struct"),
            "expected struct-hint message, got: {msg}"
        );
        assert!(
            msg.contains("[int, string]"),
            "message should render the offending tuple type, got: {msg}"
        );
    }

    #[test]
    fn check_tuple_against_accepts_homogeneous_int_pair() {
        let mut engine = super::super::TypeInferenceEngine::new();
        let elements = vec![
            Expr::Literal(shape_ast::ast::Literal::Int(3), Default::default()),
            Expr::Literal(shape_ast::ast::Literal::Int(4), Default::default()),
        ];
        let elem_types = vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("int".to_string()),
        ];
        assert!(
            engine.check_tuple_against(&elements, &elem_types).is_ok(),
            "homogeneous `[int, int]` must type-check"
        );
    }

    #[test]
    fn test_extract_object_fields_from_unknown_returns_empty() {
        let engine = super::super::TypeInferenceEngine::new();
        let ty = BuiltinTypes::number();
        let fields = engine.extract_object_fields(&ty);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_object_fields_from_reference_via_alias() {
        let mut engine = super::super::TypeInferenceEngine::new();
        // Register a type alias: type Point = { x: int, y: int }
        engine.env.define_type_alias(
            "Point",
            &TypeAnnotation::Object(vec![
                shape_ast::ast::ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                shape_ast::ast::ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ]),
            None,
        );

        let ty = Type::Concrete(TypeAnnotation::Reference("Point".into()));
        let fields = engine.extract_object_fields(&ty);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[1].name, "y");
    }
}
