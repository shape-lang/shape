//! ADR-009 C3 #14 (slice 1) — the G9 pseudo-tuple core: ONE traversal, two
//! faces (the construction-time usage VALIDATOR and the specialization-time
//! REWRITER).
//!
//! # The G9 ruling this enforces
//!
//! C3-G9 resolved the Args carrier as a SPECIALIZATION-RESOLVED PSEUDO-TUPLE:
//! `args[i]` / `args.length` are TEMPLATE-LEVEL constructs. No tuple value ever
//! exists at runtime — at specialization ([`resolve_pseudo_tuple`], S1c
//! stage 4), a constant-index `args[i]` resolves to the target's i-th typed
//! parameter slot, `args.length` resolves to a constant, and a mutation-return
//! (`return args` / a final bare `args`) specializes to a COMPILER-INTERNAL
//! per-target aggregate at the weave boundary (never user-visible; never the
//! boxed `NewArray` path).
//!
//! # One walker, two faces (binding invariant — do not fork)
//!
//! This module is the single traversal core. [`Face::Validate`] is the
//! construction face ([`validate_pseudo_tuple_uses`], run from
//! `CheckedTemplateBuilder::finish()`); [`Face::Rewrite`] is the
//! specialization face ([`resolve_pseudo_tuple`], run from the
//! monomorphization ride under a [`TemplateSpecializationPlan`]). BOTH faces
//! share every shape classifier and every named rejection — there is no
//! second, drifting walker; the validate face takes `&mut` for traversal
//! uniformity only and never mutates (every mutation sits behind
//! `Face::Rewrite`). The traversal mirrors the exhaustive Statement/Expr
//! skeleton of `monomorphization/substitution.rs` (the precedent full-AST
//! walk): every `match` is exhaustive with no catch-all arm, so a new AST
//! variant is a compile error here, exactly like the substitution walker.
//!
//! # The G9 aggregate is ADR-006-ordinary (compliance statement)
//!
//! The multi-parameter mutation carrier the rewriter emits is the ORDINARY
//! inline-schema TypedObject the ordinary pipeline emits for a typed object
//! literal — `HeapValue::TypedObject(Arc<TypedObjectStorage>)` per ADR-006
//! §2.3. Its fields are FULLY typed from the target's declared annotations
//! (never `FieldType::Any` — the legacy ctx schema at
//! `functions_annotations.rs:4276-4280` is the counter-example and a C3-G7
//! deletion target). No `Box<HeapValue>`, no new `HeapKind`, no new
//! discriminator, no `KindedSlot` in the typed VM↔JIT slot ABI — the
//! aggregate flows through the ordinary function-return path. It is
//! compiler-internal: the field names `a0..aN-1` are the weave contract,
//! never user-visible (C3-G9; #63 tracks a first-class tuple surface).
//!
//! # Known validation boundaries (named, deliberate)
//!
//! - **Out-of-range constant indices are NOT construction-checkable** — arity
//!   is a property of the frozen TARGET, which construction never sees. The
//!   rewrite face rejects `args[7]` against a 2-parameter target with a named
//!   rejection quoting the index and the target's arity + signature; the
//!   specialization seam wraps it in the two-signature application-site
//!   attribution.
//! - **Interpolated f-string contents are not scanned.** A `Literal::
//!   FormattedString` carries its interpolation as raw text until emission; a
//!   pseudo-tuple reference inside one (`f"{args}"`) is caught downstream by
//!   ordinary identifier resolution after the pseudo-tuple has resolved away
//!   (the name no longer exists), never silently honored.
//!
//! # The `__c3_` reserved prefix
//!
//! The rewrite face mints specialization-internal names under the `__c3_`
//! prefix (`__c3_p{i}` parameters, `__c3_arg_{i}` mutable locals). The walker
//! rejects ANY identifier carrying that prefix in a template body so a minted
//! name can never collide with (or capture) user spelling — the same
//! internal-name discipline as the legacy wrapper's `__args`/`__result`/`__ctx`
//! locals (`compile_annotation_wrapper`, `functions_annotations.rs:4232-4234`),
//! but enforced at construction instead of relied on by convention. Minted
//! nodes are inserted AFTER the walk (prologue/params) or as terminal
//! replacements the traversal never revisits, so the reserved-prefix check
//! never fires on the rewriter's own output.

use shape_ast::ast::expr_helpers::{BlockItem, QueryClause};
use shape_ast::ast::expressions::{EnumConstructorPayload, Expr, ObjectEntry};
use shape_ast::ast::functions::{Annotation, FunctionDef, FunctionParameter};
use shape_ast::ast::literals::Literal;
use shape_ast::ast::patterns::{DestructurePattern, Pattern, PatternConstructorFields};
use shape_ast::ast::program::{Assignment, OwnershipModifier, VarKind, VariableDecl};
use shape_ast::ast::span::{Span, Spanned};
use shape_ast::ast::statements::{ForInit, Statement};
use shape_ast::ast::types::{ExtendStatement, MethodDef, ObjectTypeField, TypeAnnotation};
use shape_ast::ast::windows::{WindowExpr, WindowFunction};
use shape_ast::error::{Result, ShapeError};

use super::MutationCarrier;

/// The reserved specialization-internal identifier prefix (see module docs).
pub(in crate::compiler) const RESERVED_SPECIALIZATION_PREFIX: &str = "__c3_";

/// The minted parameter name for the target's i-th typed slot (`__c3_p{i}`).
fn slot_param_name(index: usize) -> String {
    format!("{RESERVED_SPECIALIZATION_PREFIX}p{index}")
}

/// The minted mutable local backing the i-th slot (`__c3_arg_{i}`).
fn slot_local_name(index: usize) -> String {
    format!("{RESERVED_SPECIALIZATION_PREFIX}arg_{index}")
}

/// Everything the rewrite face needs to resolve one polymorphic BEFORE
/// template body against one frozen target (C3-G9/G10). Built at the
/// specialization seam (`template_specialization::specialize_template`) and
/// consumed by the monomorphization ride
/// (`cache.rs::ensure_monomorphic_template_specialization`) — the plan is an
/// explicit parameter end to end; no ambient state.
#[derive(Debug)]
pub(in crate::compiler) struct TemplateSpecializationPlan {
    /// The template's pseudo-tuple parameter spelling (`args`).
    pub(in crate::compiler) args_param: String,
    /// The template's type parameter spelling (`Args`).
    pub(in crate::compiler) type_param: String,
    /// The frozen target's parameters: display name + AST-side declared
    /// annotation, in signature order (slice-0 §7.4 — AST side, never the
    /// freeze round-trip).
    pub(in crate::compiler) target_params: Vec<(String, TypeAnnotation)>,
    /// How the mutated pack flows back (C3-G9): `Single` for a 1-ary target,
    /// the compiler-internal `Aggregate` otherwise.
    pub(in crate::compiler) carrier: MutationCarrier,
}

impl TemplateSpecializationPlan {
    fn arity(&self) -> usize {
        self.target_params.len()
    }

    /// The expression a mutation-return specializes to: the bare typed local
    /// for `Single`, the inline-schema typed object literal for `Aggregate`
    /// (see the module-docs ADR-006 compliance statement).
    fn carrier_expr(&self, span: Span) -> Expr {
        match &self.carrier {
            MutationCarrier::Single { .. } => Expr::Identifier(slot_local_name(0), span),
            MutationCarrier::Aggregate { fields } => Expr::Object(
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, (name, _))| ObjectEntry::Field {
                        key: name.clone(),
                        value: Expr::Identifier(slot_local_name(index), span),
                        type_annotation: None,
                    })
                    .collect(),
                span,
            ),
        }
    }

    /// The specialized return annotation: the target's one parameter type for
    /// `Single`, the fully-typed inline object schema for `Aggregate`. The
    /// transient post-substitution `Tuple` annotation is REPLACED here — it
    /// never reaches checking or emission (C3-G9).
    fn carrier_return_annotation(&self) -> TypeAnnotation {
        match &self.carrier {
            MutationCarrier::Single { annotation } => annotation.clone(),
            MutationCarrier::Aggregate { fields } => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|(name, annotation)| ObjectTypeField {
                        name: name.clone(),
                        optional: false,
                        type_annotation: annotation.clone(),
                        annotations: Vec::new(),
                    })
                    .collect(),
            ),
        }
    }

    /// Render the target's parameter list for the out-of-range rejection
    /// (`a: int, b: number`).
    fn render_target_params(&self) -> String {
        self.target_params
            .iter()
            .map(|(name, annotation)| format!("{name}: {}", annotation.to_type_string()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

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
/// silently change what the rewrite face targets) and any identifier with the
/// reserved `__c3_` prefix.
///
/// Called from `CheckedTemplateBuilder::finish()` for
/// `TemplateSig::PolymorphicArgs` only — concrete bodies and polymorphic
/// AFTER bodies (`result`) carry ordinary values with no pseudo-tuple
/// surface. The `&mut` is traversal-uniformity with the rewrite face only;
/// the validate face never mutates.
pub(in crate::compiler) fn validate_pseudo_tuple_uses(
    body: &mut [Statement],
    args_param: &str,
    type_param: &str,
) -> Result<()> {
    let scan = Scan {
        args_param,
        type_param,
        face: Face::Validate,
    };
    walk_template_body(&scan, body)
}

/// The rewrite face (C3-G9 resolution): resolve every pseudo-tuple construct
/// in an already-substituted specialized template def against the plan's
/// frozen target. Transform, in order:
///
/// 1. Walk the body with [`Face::Rewrite`]: constant-index `args[i]` (read
///    and assignment-target) → the minted local `__c3_arg_{i}` (a constant
///    index outside `[0, N)` is a NAMED rejection quoting the index and the
///    target's arity + signature); `args.length` → the constant `N`;
///    `return args` / the final bare-`args` tail → the carrier expression.
///    Any residual `args`/`type_param` use named-rejects via the SAME shared
///    core the construction face runs.
/// 2. Replace the single `args` parameter with N parameters `__c3_p{i}`
///    typed with the target's AST-side annotations.
/// 3. Prepend the prologue `let mut __c3_arg_{i} = __c3_p{i}` per parameter
///    (uniform mutability without assuming parameter assignability).
/// 4. Replace the (substituted-to-`Tuple`) return annotation with the
///    carrier annotation — the transient `Tuple` never reaches checking or
///    emission.
///
/// The walk runs BEFORE the prologue/parameter minting so the reserved-prefix
/// check never sees the rewriter's own output.
pub(in crate::compiler) fn resolve_pseudo_tuple(
    def: &mut FunctionDef,
    plan: &TemplateSpecializationPlan,
) -> Result<()> {
    let arity = plan.arity();
    let internal = |message: String| ShapeError::RuntimeError {
        message,
        location: None,
    };
    if arity == 0 {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple for `{}` received a zero-parameter target \
             plan; the specialization seam rejects zero-parameter before-targets before \
             building a plan",
            def.name
        )));
    }
    let carrier_consistent = match &plan.carrier {
        MutationCarrier::Single { .. } => arity == 1,
        MutationCarrier::Aggregate { fields } => arity > 1 && fields.len() == arity,
    };
    if !carrier_consistent {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple for `{}` received an inconsistent plan \
             (arity {arity} vs carrier {:?}); the specialization seam derives the carrier \
             from the same target parameter list",
            def.name, plan.carrier
        )));
    }
    if def.params.len() != 1
        || def.params[0].simple_name() != Some(plan.args_param.as_str())
    {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple expected `{}` to carry exactly the one \
             pseudo-tuple parameter `{}`; a PolymorphicArgs template classifies with exactly \
             that shape at construction",
            def.name, plan.args_param
        )));
    }

    // (1) The rewrite walk — same core, second face.
    let scan = Scan {
        args_param: &plan.args_param,
        type_param: &plan.type_param,
        face: Face::Rewrite(plan),
    };
    walk_template_body(&scan, &mut def.body)?;

    // (2) Per-slot minted parameters, typed from the target's AST side.
    def.params = plan
        .target_params
        .iter()
        .enumerate()
        .map(|(index, (_, annotation))| FunctionParameter {
            pattern: DestructurePattern::Identifier(slot_param_name(index), Span::default()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(annotation.clone()),
            default_value: None,
        })
        .collect();

    // (3) The mutable-local prologue.
    let mut body = Vec::with_capacity(arity + def.body.len());
    for index in 0..arity {
        body.push(Statement::VariableDecl(
            VariableDecl {
                kind: VarKind::Let,
                is_mut: true,
                pattern: DestructurePattern::Identifier(slot_local_name(index), Span::default()),
                type_annotation: None,
                value: Some(Expr::Identifier(slot_param_name(index), Span::default())),
                ownership: OwnershipModifier::Inferred,
            },
            Span::default(),
        ));
    }
    body.append(&mut def.body);
    def.body = body;

    // (4) The carrier return annotation replaces the transient Tuple. The
    // production caller hands in a substitution output whose `type_params`
    // are already cleared; clear defensively so the resolved def is concrete
    // either way (the polymorphism has resolved away).
    def.return_type = Some(plan.carrier_return_annotation());
    def.type_params = None;
    Ok(())
}

/// The shared top-level driver both faces run: per-statement traversal with
/// the implicit-return tail interception (a FINAL bare `args` expression
/// statement at the TOP LEVEL of the body is the mutation-return spelling).
fn walk_template_body(scan: &Scan<'_>, body: &mut [Statement]) -> Result<()> {
    let last = body.len().checked_sub(1);
    for (i, stmt) in body.iter_mut().enumerate() {
        if Some(i) == last {
            let is_tail_bare_args = matches!(
                stmt,
                Statement::Expression(Expr::Identifier(name, _), _)
                    if name.as_str() == scan.args_param
            );
            if is_tail_bare_args {
                if let Face::Rewrite(plan) = scan.face {
                    let Statement::Expression(expr, _) = stmt else {
                        unreachable!("guarded by is_tail_bare_args");
                    };
                    let span = expr.span();
                    *expr = plan.carrier_expr(span);
                }
                continue;
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

/// Which face of the single walker is running (see the module docs).
#[derive(Clone, Copy)]
enum Face<'a> {
    /// Construction-time usage validation — classifies and rejects, never
    /// mutates.
    Validate,
    /// Specialization-time resolution — the same classification plus the G9
    /// node replacements.
    Rewrite(&'a TemplateSpecializationPlan),
}

/// What the template-body interception decided for one expression node.
enum Intercept {
    /// Not a pseudo-tuple form — continue the generic traversal.
    No,
    /// A legal pseudo-tuple form; the validate face keeps the node.
    LegalKeep,
    /// A legal pseudo-tuple form; the rewrite face replaces the node.
    Replace(Expr),
}

struct Scan<'a> {
    args_param: &'a str,
    type_param: &'a str,
    face: Face<'a>,
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

    /// Rewrite face only: a constant index with no parameter slot on THIS
    /// target. Quotes the index and the target's arity + signature (the
    /// specialization seam adds the two-signature application-site frame).
    fn reject_index_out_of_range(
        &self,
        index: i64,
        plan: &TemplateSpecializationPlan,
    ) -> ShapeError {
        let arity = plan.arity();
        reject(format!(
            "the `{args}` pseudo-tuple index {index} is out of range for this target: the \
             target declares {arity} parameter{plural} ({params}), so constant indices resolve \
             only in 0..{arity}",
            args = self.args_param,
            plural = if arity == 1 { "" } else { "s" },
            params = plan.render_target_params(),
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

    /// The minted local for a range-checked constant index (rewrite face).
    fn checked_slot_local(
        &self,
        index: i64,
        plan: &TemplateSpecializationPlan,
    ) -> Result<String> {
        if index < 0 || (index as usize) >= plan.arity() {
            return Err(self.reject_index_out_of_range(index, plan));
        }
        Ok(slot_local_name(index as usize))
    }

    // ---------------------------------------------------------------------
    // Statements
    // ---------------------------------------------------------------------

    fn statements(&self, stmts: &mut [Statement], mode: ScanMode) -> Result<()> {
        for stmt in stmts.iter_mut() {
            self.statement(stmt, mode)?;
        }
        Ok(())
    }

    fn statement(&self, stmt: &mut Statement, mode: ScanMode) -> Result<()> {
        match stmt {
            Statement::Return(value, _) => {
                // `return args` — the mutation-return spelling (template body
                // only; inside a closure it is an occurrence like any other).
                if mode == ScanMode::TemplateBody {
                    if let Some(inner) = value {
                        if self.is_args_identifier(inner) {
                            if let Face::Rewrite(plan) = self.face {
                                let span = inner.span();
                                *inner = plan.carrier_expr(span);
                            }
                            return Ok(());
                        }
                    }
                }
                self.opt_expr(value.as_mut(), mode)
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => Ok(()),
            Statement::VariableDecl(decl, _) => self.variable_decl(decl, mode),
            Statement::Assignment(assign, _) => self.assignment(assign, mode),
            Statement::Expression(expr, _) => self.expr(expr, mode),
            Statement::For(for_loop, _) => {
                match &mut for_loop.init {
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
                self.statements(&mut for_loop.body, mode)
            }
            Statement::While(while_loop, _) => {
                self.expr(&mut while_loop.condition, mode)?;
                self.statements(&mut while_loop.body, mode)
            }
            Statement::If(if_stmt, _) => {
                self.expr(&mut if_stmt.condition, mode)?;
                self.statements(&mut if_stmt.then_body, mode)?;
                if let Some(else_body) = &mut if_stmt.else_body {
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

    fn variable_decl(&self, decl: &mut VariableDecl, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_binding(&decl.pattern, mode)?;
        if let Some(annotation) = &decl.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(decl.value.as_mut(), mode)
    }

    fn assignment(&self, assign: &mut Assignment, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_assign_target(&assign.pattern, mode)?;
        self.expr(&mut assign.value, mode)
    }

    fn extend(&self, ext: &mut ExtendStatement, mode: ScanMode) -> Result<()> {
        for method in &mut ext.methods {
            self.method_def(method, mode)?;
        }
        Ok(())
    }

    fn method_def(&self, method: &mut MethodDef, mode: ScanMode) -> Result<()> {
        self.check_reserved(&method.name)?;
        for annotation in &mut method.annotations {
            self.annotation_args(annotation, mode)?;
        }
        for param in &mut method.params {
            self.function_parameter(param, mode)?;
        }
        if let Some(when) = &mut method.when_clause {
            self.expr(when, mode)?;
        }
        if let Some(ret) = &method.return_type {
            self.type_annotation(ret, mode)?;
        }
        self.statements(&mut method.body, mode)
    }

    fn function_parameter(&self, param: &mut FunctionParameter, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_binding(&param.pattern, mode)?;
        if let Some(annotation) = &param.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(param.default_value.as_mut(), mode)
    }

    fn annotation_args(&self, annotation: &mut Annotation, mode: ScanMode) -> Result<()> {
        for arg in &mut annotation.args {
            self.expr(arg, mode)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Patterns (never rewritten — shared borrows suffice)
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
    // Type annotations (never rewritten — shared borrows suffice)
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

    fn opt_expr(&self, expr: Option<&mut Expr>, mode: ScanMode) -> Result<()> {
        match expr {
            Some(inner) => self.expr(inner, mode),
            None => Ok(()),
        }
    }

    fn exprs(&self, exprs: &mut [Expr], mode: ScanMode) -> Result<()> {
        for expr in exprs.iter_mut() {
            self.expr(expr, mode)?;
        }
        Ok(())
    }

    fn named_exprs(&self, entries: &mut [(String, Expr)], mode: ScanMode) -> Result<()> {
        for (_, value) in entries.iter_mut() {
            self.expr(value, mode)?;
        }
        Ok(())
    }

    /// The legal `args[<int literal>]` shape, checked when the receiver is
    /// the pseudo-tuple identifier in `TemplateBody` mode. Returns the
    /// constant index value, or the named rejection for slices and
    /// non-constant indices.
    fn args_index_access(&self, index: &Expr, end_index: Option<&Expr>) -> Result<i64> {
        if end_index.is_some() {
            return Err(self.reject_slice());
        }
        match index {
            Expr::Literal(Literal::Int(value), _) => Ok(*value),
            _ => Err(self.reject_non_constant_index()),
        }
    }

    /// Classify one expression node against the template-body pseudo-tuple
    /// surface. Shared by both faces: legality (and its named rejections) is
    /// decided HERE once; the faces differ only in keep-vs-replace.
    fn intercept_template_body_expr(&self, expr: &Expr) -> Result<Intercept> {
        match expr {
            Expr::PropertyAccess {
                object,
                property,
                optional,
                span,
            } if self.is_args_identifier(object) => {
                if property != "length" {
                    return Err(self.reject_other_property(property));
                }
                if *optional {
                    return Err(self.reject_optional_access());
                }
                Ok(match self.face {
                    Face::Validate => Intercept::LegalKeep,
                    Face::Rewrite(plan) => Intercept::Replace(Expr::Literal(
                        Literal::Int(plan.arity() as i64),
                        *span,
                    )),
                })
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                span,
            } if self.is_args_identifier(object) => {
                let slot_index = self.args_index_access(index, end_index.as_deref())?;
                Ok(match self.face {
                    Face::Validate => Intercept::LegalKeep,
                    Face::Rewrite(plan) => Intercept::Replace(Expr::Identifier(
                        self.checked_slot_local(slot_index, plan)?,
                        *span,
                    )),
                })
            }
            _ => Ok(Intercept::No),
        }
    }

    fn expr(&self, expr: &mut Expr, mode: ScanMode) -> Result<()> {
        if mode == ScanMode::TemplateBody {
            match self.intercept_template_body_expr(expr)? {
                Intercept::No => {}
                Intercept::LegalKeep => return Ok(()),
                Intercept::Replace(replacement) => {
                    *expr = replacement;
                    return Ok(());
                }
            }
        }
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
                        if name.as_str() == self.args_param {
                            // Bare `args` in a value position (the legal
                            // return/tail spellings are handled by the
                            // callers before recursion reaches here).
                            return Err(self.reject_bare_value());
                        }
                        Ok(())
                    }
                    ScanMode::ClosureInterior => {
                        if name.as_str() == self.args_param || name.as_str() == self.type_param {
                            return Err(self.reject_closure_occurrence(name));
                        }
                        Ok(())
                    }
                }
            }

            Expr::TypeSyntax(annotation, _) => self.type_annotation(annotation, mode),

            Expr::DataRelativeAccess { reference, .. } => self.expr(reference, mode),

            // The args-receiver cases were intercepted above (TemplateBody);
            // here only ordinary receivers (and closure-interior occurrences,
            // which reject at the Identifier leaf) remain.
            Expr::PropertyAccess { object, .. } => self.expr(object, mode),

            Expr::IndexAccess {
                object,
                index,
                end_index,
                span: _,
            } => {
                self.expr(object, mode)?;
                self.expr(index, mode)?;
                self.opt_expr(end_index.as_deref_mut(), mode)
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
                self.opt_expr(else_expr.as_deref_mut(), mode)
            }

            Expr::Object(entries, _) => {
                for entry in entries.iter_mut() {
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
                for clause in &mut comp.clauses {
                    self.destructure_pattern_binding(&clause.pattern, mode)?;
                    self.expr(&mut clause.iterable, mode)?;
                    if let Some(filter) = &mut clause.filter {
                        self.expr(filter, mode)?;
                    }
                }
                self.expr(&mut comp.element, mode)
            }

            Expr::Block(block, _) => {
                for item in &mut block.items {
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
                    for value in overrides.values_mut() {
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
                for param in params.iter_mut() {
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
                self.expr(&mut if_expr.condition, mode)?;
                self.expr(&mut if_expr.then_branch, mode)?;
                self.opt_expr(if_expr.else_branch.as_deref_mut(), mode)
            }

            Expr::While(while_expr, _) => {
                self.expr(&mut while_expr.condition, mode)?;
                self.expr(&mut while_expr.body, mode)
            }

            Expr::For(for_expr, _) => {
                self.match_pattern(&for_expr.pattern, mode)?;
                self.expr(&mut for_expr.iterable, mode)?;
                self.expr(&mut for_expr.body, mode)
            }

            Expr::Loop(loop_expr, _) => self.expr(&mut loop_expr.body, mode),

            Expr::Let(let_expr, _) => {
                self.match_pattern(&let_expr.pattern, mode)?;
                if let Some(annotation) = &let_expr.type_annotation {
                    self.type_annotation(annotation, mode)?;
                }
                self.opt_expr(let_expr.value.as_deref_mut(), mode)?;
                self.expr(&mut let_expr.body, mode)
            }

            Expr::Assign(assign_expr, _) => {
                // `args[<int literal>] = expr` — the legal per-slot mutation
                // target. The target's own index legality is checked with the
                // same core as the read path; the rewrite face swaps the
                // target for the minted local.
                if mode == ScanMode::TemplateBody {
                    let intercepted = match assign_expr.target.as_ref() {
                        Expr::IndexAccess {
                            object,
                            index,
                            end_index,
                            span,
                        } if self.is_args_identifier(object) => {
                            Some((self.args_index_access(index, end_index.as_deref())?, *span))
                        }
                        _ => None,
                    };
                    if let Some((slot_index, span)) = intercepted {
                        if let Face::Rewrite(plan) = self.face {
                            *assign_expr.target = Expr::Identifier(
                                self.checked_slot_local(slot_index, plan)?,
                                span,
                            );
                        }
                        return self.expr(&mut assign_expr.value, mode);
                    }
                }
                self.expr(&mut assign_expr.target, mode)?;
                self.expr(&mut assign_expr.value, mode)
            }

            Expr::Break(value, _) => self.opt_expr(value.as_deref_mut(), mode),

            Expr::Return(value, _) => {
                // The parser's expression-position `return` twin: the same
                // authored `return args` spelling (see the fn docs).
                if mode == ScanMode::TemplateBody {
                    if let Some(inner) = value.as_deref_mut() {
                        if self.is_args_identifier(inner) {
                            if let Face::Rewrite(plan) = self.face {
                                let span = inner.span();
                                *inner = plan.carrier_expr(span);
                            }
                            return Ok(());
                        }
                    }
                }
                self.opt_expr(value.as_deref_mut(), mode)
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
                self.expr(&mut match_expr.scrutinee, mode)?;
                for arm in &mut match_expr.arms {
                    self.match_pattern(&arm.pattern, mode)?;
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard, mode)?;
                    }
                    self.expr(&mut arm.body, mode)?;
                }
                Ok(())
            }

            Expr::Range { start, end, .. } => {
                self.opt_expr(start.as_deref_mut(), mode)?;
                self.opt_expr(end.as_deref_mut(), mode)
            }

            Expr::TimeframeContext { expr: inner, .. } => self.expr(inner, mode),

            Expr::TryOperator(inner, _) => self.expr(inner, mode),

            Expr::UsingImpl { expr: inner, .. } => self.expr(inner, mode),

            Expr::SimulationCall { params, span: _, .. } => self.named_exprs(params, mode),

            Expr::WindowExpr(window, _) => self.window_expr(window, mode),

            Expr::FromQuery(query, _) => {
                self.check_binding_name(&query.variable, mode)?;
                self.expr(&mut query.source, mode)?;
                for clause in &mut query.clauses {
                    match clause {
                        QueryClause::Where(cond) => self.expr(cond, mode)?,
                        QueryClause::OrderBy(specs) => {
                            for spec in specs.iter_mut() {
                                self.expr(&mut spec.key, mode)?;
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
                self.expr(&mut query.select, mode)
            }

            Expr::StructLiteral { fields, .. } => self.named_exprs(fields, mode),

            Expr::Await(inner, _) => self.expr(inner, mode),

            Expr::Join(join, _) => {
                for branch in &mut join.branches {
                    for annotation in &mut branch.annotations {
                        self.annotation_args(annotation, mode)?;
                    }
                    self.expr(&mut branch.expr, mode)?;
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
                self.expr(&mut async_let.expr, mode)
            }

            Expr::AsyncScope(inner, _) => self.expr(inner, mode),

            Expr::Comptime(stmts, _) => self.statements(stmts, mode),

            Expr::ComptimeFor(comp_for, _) => {
                for witness in &comp_for.witnesses {
                    self.check_binding_name(witness, mode)?;
                }
                self.check_binding_name(&comp_for.variable, mode)?;
                self.expr(&mut comp_for.iterable, mode)?;
                self.statements(&mut comp_for.body, mode)
            }

            Expr::Reference { expr: inner, .. } => self.expr(inner, mode),

            Expr::TableRows(rows, _) => {
                for row in rows.iter_mut() {
                    self.exprs(row, mode)?;
                }
                Ok(())
            }
        }
    }

    fn window_expr(&self, window: &mut WindowExpr, mode: ScanMode) -> Result<()> {
        match &mut window.function {
            WindowFunction::Lag { expr, default, .. }
            | WindowFunction::Lead { expr, default, .. } => {
                self.expr(expr, mode)?;
                self.opt_expr(default.as_deref_mut(), mode)?;
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
            WindowFunction::Count(expr) => self.opt_expr(expr.as_deref_mut(), mode)?,
        }
        self.exprs(&mut window.over.partition_by, mode)?;
        if let Some(order_by) = &mut window.over.order_by {
            for (expr, _) in &mut order_by.columns {
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

    fn def_of(src: &str) -> FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn body_of(src: &str) -> Vec<Statement> {
        def_of(src).body
    }

    fn validate(src: &str) -> Result<()> {
        let mut body = body_of(src);
        validate_pseudo_tuple_uses(&mut body, "args", "Args")
    }

    fn expect_reject(src: &str, needle: &str) {
        let err = validate(src).expect_err("fixture must be rejected");
        assert!(
            err.to_string().contains(needle),
            "expected rejection containing {needle:?}, got: {err}"
        );
    }

    fn int_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("int".into())
    }

    fn number_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("number".into())
    }

    /// A 2-ary `(int, number)` aggregate plan — the mod.rs seam's shape.
    fn aggregate_plan() -> TemplateSpecializationPlan {
        TemplateSpecializationPlan {
            args_param: "args".into(),
            type_param: "Args".into(),
            target_params: vec![("a".into(), int_ann()), ("b".into(), number_ann())],
            carrier: MutationCarrier::Aggregate {
                fields: vec![("a0".into(), int_ann()), ("a1".into(), number_ann())],
            },
        }
    }

    /// A 1-ary `(int)` Single plan.
    fn single_plan() -> TemplateSpecializationPlan {
        TemplateSpecializationPlan {
            args_param: "args".into(),
            type_param: "Args".into(),
            target_params: vec![("a".into(), int_ann())],
            carrier: MutationCarrier::Single {
                annotation: int_ann(),
            },
        }
    }

    fn resolve(src: &str, plan: &TemplateSpecializationPlan) -> Result<FunctionDef> {
        let mut def = def_of(src);
        resolve_pseudo_tuple(&mut def, plan)?;
        Ok(def)
    }

    fn expect_resolve_reject(src: &str, plan: &TemplateSpecializationPlan, needle: &str) {
        let err = resolve(src, plan).expect_err("fixture must be rejected by the rewrite face");
        assert!(
            err.to_string().contains(needle),
            "expected rewrite-face rejection containing {needle:?}, got: {err}"
        );
    }

    // =====================================================================
    // Validate face (construction-time) — behavior unchanged from S1b.
    // =====================================================================

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

    // =====================================================================
    // Rewrite face (specialization-time) — the C3-G9 resolution.
    // =====================================================================

    // The full aggregate transform: params minted + typed, prologue
    // prepended, index read/write and `.length` resolved, the return
    // rewritten to the inline-schema object, and the return annotation
    // replaced (no Tuple survives).
    #[test]
    fn aggregate_resolution_transforms_the_full_surface() {
        let def = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] + 1
    let n = args.length
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect("the legal surface resolves");

        // (2) minted, typed parameters.
        assert_eq!(def.params.len(), 2);
        assert_eq!(def.params[0].simple_name(), Some("__c3_p0"));
        assert_eq!(def.params[1].simple_name(), Some("__c3_p1"));
        assert_eq!(def.params[0].type_annotation, Some(int_ann()));
        assert_eq!(def.params[1].type_annotation, Some(number_ann()));

        // (3) the prologue: let mut __c3_arg_i = __c3_pi.
        for (index, stmt) in def.body[..2].iter().enumerate() {
            let Statement::VariableDecl(decl, _) = stmt else {
                panic!("expected prologue decl at {index}, got {stmt:?}");
            };
            assert_eq!(decl.kind, VarKind::Let);
            assert!(decl.is_mut, "prologue locals are uniformly mutable");
            assert_eq!(
                decl.pattern,
                DestructurePattern::Identifier(slot_local_name(index), Span::default())
            );
            assert_eq!(
                decl.value,
                Some(Expr::Identifier(slot_param_name(index), Span::default()))
            );
        }

        // (1) the per-slot assignment target and read both resolved.
        let Statement::Expression(Expr::Assign(assign, _), _) = &def.body[2] else {
            panic!("expected the rewritten assignment, got {:?}", def.body[2]);
        };
        assert!(
            matches!(assign.target.as_ref(), Expr::Identifier(name, _) if name == "__c3_arg_0"),
            "assign target must resolve to the minted local: {:?}",
            assign.target
        );
        assert!(
            format!("{:?}", assign.value).contains("__c3_arg_0"),
            "the read side must resolve to the minted local: {:?}",
            assign.value
        );

        // `.length` → the arity constant.
        let Statement::VariableDecl(n_decl, _) = &def.body[3] else {
            panic!("expected the length decl, got {:?}", def.body[3]);
        };
        assert!(
            matches!(n_decl.value, Some(Expr::Literal(Literal::Int(2), _))),
            "args.length must resolve to the constant target arity: {:?}",
            n_decl.value
        );

        // The mutation-return → the aggregate object literal.
        let Statement::Return(Some(Expr::Object(entries, _)), _) = &def.body[4] else {
            panic!("expected the aggregate return, got {:?}", def.body[4]);
        };
        assert_eq!(entries.len(), 2);
        for (index, entry) in entries.iter().enumerate() {
            let ObjectEntry::Field { key, value, .. } = entry else {
                panic!("expected a plain field, got {entry:?}");
            };
            assert_eq!(key, &format!("a{index}"));
            assert!(
                matches!(value, Expr::Identifier(name, _) if name == &slot_local_name(index))
            );
        }

        // (4) the return annotation is the fully-typed inline object schema —
        // the transient Tuple annotation never survives resolution.
        let Some(TypeAnnotation::Object(fields)) = &def.return_type else {
            panic!("expected the object return annotation, got {:?}", def.return_type);
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "a0");
        assert_eq!(fields[0].type_annotation, int_ann());
        assert_eq!(fields[1].name, "a1");
        assert_eq!(fields[1].type_annotation, number_ann());
        assert!(
            !matches!(def.return_type, Some(TypeAnnotation::Tuple(_))),
            "no Tuple annotation may survive resolution"
        );
        assert!(def.type_params.is_none(), "the resolved def is concrete");
    }

    // Single-carrier resolution: the mutation-return (here the bare-args
    // TAIL spelling) becomes the bare minted local; the return annotation is
    // the target's one parameter type.
    #[test]
    fn single_resolution_rewrites_tail_to_the_minted_local() {
        let def = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] * 3
    args
}
"#,
            &single_plan(),
        )
        .expect("the single-carrier surface resolves");

        assert_eq!(def.params.len(), 1);
        assert_eq!(def.params[0].simple_name(), Some("__c3_p0"));
        let Statement::Expression(tail, _) = def.body.last().expect("body has a tail") else {
            panic!("expected the rewritten tail expression");
        };
        assert!(
            matches!(tail, Expr::Identifier(name, _) if name == "__c3_arg_0"),
            "the bare-args tail must resolve to the minted local: {tail:?}"
        );
        assert_eq!(def.return_type, Some(int_ann()));
    }

    // NEGATIVE (rewrite face): an out-of-range constant READ quotes the
    // index and the target's arity + signature.
    #[test]
    fn out_of_range_read_is_rejected_naming_index_and_arity() {
        let err = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[7]
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect_err("an out-of-range index must be rejected at specialization");
        let message = err.to_string();
        assert!(
            message.contains("index 7 is out of range"),
            "must quote the index: {message}"
        );
        assert!(
            message.contains("declares 2 parameters"),
            "must quote the target arity: {message}"
        );
        assert!(
            message.contains("a: int, b: number"),
            "must quote the target signature: {message}"
        );
    }

    // NEGATIVE (rewrite face): the same out-of-range rule on the ASSIGNMENT
    // target.
    #[test]
    fn out_of_range_assignment_target_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args[2] = 1
    return args
}
"#,
            &aggregate_plan(),
            "index 2 is out of range",
        );
    }

    // FAIL-CLOSED (rewrite face): the shared core fires the SAME named
    // rejections for shapes construction already rejects — a non-constant
    // index …
    #[test]
    fn rewrite_face_rejects_non_constant_index_via_the_shared_core() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let i = 0
    args[i] = 1
    return args
}
"#,
            &aggregate_plan(),
            "compile-time-constant index",
        );
    }

    // … and a closure-interior occurrence.
    #[test]
    fn rewrite_face_rejects_closure_occurrence_via_the_shared_core() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x| x + args[0]
    return args
}
"#,
            &aggregate_plan(),
            "does not cross closure boundaries",
        );
    }

    // INTERNAL INVARIANT: a def that does not carry exactly the pseudo-tuple
    // parameter is an internal error (the classifier guarantees the shape).
    #[test]
    fn resolve_on_a_mismatched_def_is_an_internal_error() {
        let mut def = def_of("fn t(x: int, y: int) -> int { return x }");
        let err = resolve_pseudo_tuple(&mut def, &aggregate_plan())
            .expect_err("a mismatched def must be an internal error");
        assert!(
            err.to_string().contains("internal error"),
            "expected the named internal invariant error: {err}"
        );
    }
}
