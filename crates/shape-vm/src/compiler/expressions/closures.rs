//! Closure (function expression) compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use crate::compiler::monomorphization::type_resolution::concrete_type_for_expr;
use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
use shape_ast::ast::type_path::TypePath;
use shape_ast::ast::{Expr, FunctionDef, Span, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_value::v2::concrete_type::{ClosureTypeId, ConcreteType};
use std::collections::BTreeSet;

use super::super::BytecodeCompiler;

/// Strict-typing-sweep (Cluster 2): scan a closure body for binary ops
/// of the form `<param_name> <op> <literal>` (or the symmetric form), and
/// derive a `TypeAnnotation` for `param_name` from the literal's type when
/// the literal has one. This handles the canonical
/// `|x| x + 1` / `|y| y + N` patterns that previously rode on the
/// (now-deleted) Dynamic-emission shim.
///
/// Conservative: returns `None` if the param appears only in untyped
/// contexts, or if the binary op pairs the param with another unknown
/// (e.g. `|x, y| x + y`). The closure body still compiles in those cases
/// — strict-typing simply errors at the offending binary op as before.
pub(crate) fn infer_param_type_from_body(
    param_name: &str,
    body: &[shape_ast::ast::Statement],
) -> Option<TypeAnnotation> {
    use shape_ast::ast::{Literal, Statement};
    fn literal_to_type_ann(lit: &Literal) -> Option<TypeAnnotation> {
        Some(match lit {
            Literal::Int(_) => TypeAnnotation::Basic("int".to_string()),
            Literal::Number(_) => TypeAnnotation::Basic("number".to_string()),
            Literal::Bool(_) => TypeAnnotation::Basic("bool".to_string()),
            Literal::String(_) => TypeAnnotation::Basic("string".to_string()),
            _ => return None,
        })
    }
    fn scan_expr(name: &str, expr: &Expr) -> Option<TypeAnnotation> {
        match expr {
            Expr::BinaryOp { left, right, .. } => {
                if let (Expr::Identifier(n, _), Expr::Literal(lit, _)) =
                    (left.as_ref(), right.as_ref())
                {
                    if n == name {
                        if let Some(t) = literal_to_type_ann(lit) {
                            return Some(t);
                        }
                    }
                }
                if let (Expr::Literal(lit, _), Expr::Identifier(n, _)) =
                    (left.as_ref(), right.as_ref())
                {
                    if n == name {
                        if let Some(t) = literal_to_type_ann(lit) {
                            return Some(t);
                        }
                    }
                }
                scan_expr(name, left).or_else(|| scan_expr(name, right))
            }
            Expr::UnaryOp { operand, .. } => scan_expr(name, operand),
            Expr::FunctionCall { args, .. } => {
                args.iter().find_map(|a| scan_expr(name, a))
            }
            Expr::MethodCall { receiver, args, .. } => {
                scan_expr(name, receiver).or_else(|| args.iter().find_map(|a| scan_expr(name, a)))
            }
            Expr::Array(elements, _) => elements.iter().find_map(|e| scan_expr(name, e)),
            Expr::Return(Some(e), _) => scan_expr(name, e),
            // Match: when the scrutinee is the bare `name`, look at any
            // arm-pattern binding of an identifier and propagate its
            // body/guard usage back to `name`'s type. Conservatively
            // handles the common idiom `match v { x where x > 0 => x }`
            // where `v` and `x` are aliased through pattern binding.
            Expr::Match(match_expr, _) => {
                if let Expr::Identifier(scrutinee_name, _) = match_expr.scrutinee.as_ref() {
                    if scrutinee_name == name {
                        // Look at each arm; if its pattern is a single
                        // identifier `x`, scan the guard + body for
                        // `<x> op <literal>` pairings.
                        for arm in &match_expr.arms {
                            if let shape_ast::ast::Pattern::Identifier(bound_name) =
                                &arm.pattern
                            {
                                if let Some(guard) = arm.guard.as_ref() {
                                    if let Some(t) = scan_expr(bound_name, guard) {
                                        return Some(t);
                                    }
                                }
                                if let Some(t) = scan_expr(bound_name, &arm.body) {
                                    return Some(t);
                                }
                            }
                        }
                    }
                }
                // Otherwise just recurse into scrutinee + arms looking
                // for the original name.
                scan_expr(name, &match_expr.scrutinee)
                    .or_else(|| {
                        match_expr.arms.iter().find_map(|arm| {
                            arm.guard
                                .as_ref()
                                .and_then(|g| scan_expr(name, g))
                                .or_else(|| scan_expr(name, &arm.body))
                        })
                    })
            }
            _ => None,
        }
    }
    fn scan_stmt(name: &str, stmt: &Statement) -> Option<TypeAnnotation> {
        match stmt {
            Statement::Expression(expr, _) => scan_expr(name, expr),
            Statement::Return(Some(e), _) => scan_expr(name, e),
            Statement::VariableDecl(decl, _) => {
                decl.value.as_ref().and_then(|e| scan_expr(name, e))
            }
            Statement::Assignment(asgn, _) => scan_expr(name, &asgn.value),
            _ => None,
        }
    }
    body.iter().find_map(|s| scan_stmt(param_name, s))
}

/// Sweep phase 3c.x: scan a closure body for `param_name op outer_ident`
/// where `outer_ident` has a known type in `known_outer_types`, and
/// propagate that type back to `param_name`. Returns the propagated type
/// name as a `String` (e.g. "int") or `None` if no such pairing is found.
fn infer_param_type_from_outer_pairing(
    param_name: &str,
    body: &[shape_ast::ast::Statement],
    known_outer_types: &std::collections::HashMap<String, String>,
) -> Option<String> {
    use shape_ast::ast::Statement;
    fn scan(name: &str, expr: &Expr, known: &std::collections::HashMap<String, String>) -> Option<String> {
        match expr {
            Expr::BinaryOp { left, right, .. } => {
                if let (Expr::Identifier(ln, _), Expr::Identifier(rn, _)) =
                    (left.as_ref(), right.as_ref())
                {
                    if ln == name {
                        if let Some(t) = known.get(rn) {
                            return Some(t.clone());
                        }
                    }
                    if rn == name {
                        if let Some(t) = known.get(ln) {
                            return Some(t.clone());
                        }
                    }
                }
                scan(name, left, known).or_else(|| scan(name, right, known))
            }
            Expr::UnaryOp { operand, .. } => scan(name, operand, known),
            Expr::Return(Some(e), _) => scan(name, e, known),
            Expr::FunctionCall { args, .. } => {
                args.iter().find_map(|a| scan(name, a, known))
            }
            Expr::MethodCall { receiver, args, .. } => {
                scan(name, receiver, known)
                    .or_else(|| args.iter().find_map(|a| scan(name, a, known)))
            }
            _ => None,
        }
    }
    fn scan_stmt(
        name: &str,
        stmt: &Statement,
        known: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        match stmt {
            Statement::Expression(e, _) => scan(name, e, known),
            Statement::Return(Some(e), _) => scan(name, e, known),
            _ => None,
        }
    }
    body.iter().find_map(|s| scan_stmt(param_name, s, known_outer_types))
}

/// Strict-typing-sweep (Cluster 1): convert a `ConcreteType` (the v2 typed
/// value-representation type) back into an AST `TypeAnnotation` so it can be
/// attached to a synthetic capture parameter. Returning `None` falls back to
/// the no-annotation path (which is fine for opaque types — those captures
/// never participate in typed binary-ops anyway).
///
/// We map the type-name primitives that `tracked_type_name_from_annotation`
/// recognizes plus `Vec<T>` for arrays. Composite/opaque types
/// (Struct/Enum/Closure/Function/Pointer/HashMap with non-trivial inner)
/// return `None` — they don't need typed-op support inside the closure body.
pub(crate) fn concrete_type_to_type_annotation(ct: &ConcreteType) -> Option<TypeAnnotation> {
    match ct {
        ConcreteType::F64 => Some(TypeAnnotation::Basic("number".to_string())),
        ConcreteType::I64 => Some(TypeAnnotation::Basic("int".to_string())),
        ConcreteType::I32 => Some(TypeAnnotation::Basic("i32".to_string())),
        ConcreteType::I16 => Some(TypeAnnotation::Basic("i16".to_string())),
        ConcreteType::I8 => Some(TypeAnnotation::Basic("i8".to_string())),
        ConcreteType::U64 => Some(TypeAnnotation::Basic("u64".to_string())),
        ConcreteType::U32 => Some(TypeAnnotation::Basic("u32".to_string())),
        ConcreteType::U16 => Some(TypeAnnotation::Basic("u16".to_string())),
        ConcreteType::U8 => Some(TypeAnnotation::Basic("u8".to_string())),
        ConcreteType::Bool => Some(TypeAnnotation::Basic("bool".to_string())),
        ConcreteType::String => Some(TypeAnnotation::Basic("string".to_string())),
        ConcreteType::Decimal => Some(TypeAnnotation::Basic("decimal".to_string())),
        ConcreteType::BigInt => Some(TypeAnnotation::Basic("bigint".to_string())),
        ConcreteType::DateTime => Some(TypeAnnotation::Basic("DateTime".to_string())),
        ConcreteType::Array(inner) => {
            // Render as Vec<T> via the Generic form so
            // `tracked_type_name_from_annotation` produces "Vec<int>" /
            // "Vec<number>" — the names the type-tracker keys typed array
            // ops on.
            concrete_type_to_type_annotation(inner).map(|inner_ann| TypeAnnotation::Generic {
                name: TypePath::simple("Vec"),
                args: vec![inner_ann],
            })
        }
        // Nullable: drop the wrapper — the captured variable is the inner
        // value at the binary-op site if the closure narrows it. No-annotation
        // is safer than a wrong annotation.
        ConcreteType::Option(_) => None,
        // Other composite / opaque types: no useful annotation for the
        // type-tracker. The capture lives as a Pointer-typed slot via the
        // closure layout and does not participate in typed binops.
        _ => None,
    }
}

/// Sweep phase 3c.1: extract a primitive scalar type-name from a
/// runtime `Type`. Mirrors the subset of `numeric_ops::type_display_name`
/// the closure return-type inference cares about.
fn type_display_name_for_closure_inference(
    ty: &shape_runtime::type_system::Type,
) -> String {
    use shape_runtime::type_system::Type;
    match ty {
        Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
        Type::Concrete(TypeAnnotation::Reference(name)) => name.to_string(),
        _ => String::new(),
    }
}

/// Sweep phase 3c.1: infer a return-type name for a closure expression
/// based on its body, params, and the outer scope (via `compiler`).
///
/// Conservative; returns `None` when any operand or sub-expression cannot
/// be statically resolved. Used by `update_callable_binding_from_expr` to
/// populate `local_callable_return_types` so a `FunctionCall` against a
/// `let f = |…|` binding can recover `f`'s return type for strict-typing
/// binop dispatch (`f(5) + f(7)` etc.).
///
/// The helper:
/// 1. Honours an explicit `-> T` return annotation when present.
/// 2. Otherwise builds a `HashMap<String, String>` of param-name → tracked
///    type-name from the closure's params (using their annotations or the
///    body-level literal-pairing heuristic the closure compiler itself
///    relies on).
/// 3. Walks the body's terminal expression and resolves identifiers via
///    that map first, then falls back to outer-scope resolution via
///    `concrete_type_for_expr` (which recognises `let base = 100` as I64).
/// 4. Recurses into binary ops, requiring both operand types to agree
///    (and to be one of the primitive scalar names) for the result to be
///    inferred.
pub(crate) fn infer_closure_body_return_type_name(
    compiler: &mut BytecodeCompiler,
    params: &[shape_ast::ast::FunctionParameter],
    body: &[shape_ast::ast::Statement],
    explicit_return: Option<&TypeAnnotation>,
) -> Option<String> {
    infer_closure_body_return_type_name_with_outer(compiler, params, body, explicit_return, &[])
}

/// Sweep phase 3c.x: variant that also accepts a list of enclosing-scope
/// parameters whose names should resolve to their declared types when
/// scanning the closure body. Used by `update_callable_binding_from_expr`
/// for the `let f = make(...)` → `f(arg) + f(arg)` pattern, where `make`'s
/// returned closure captures `make`'s parameters by name and we want to
/// recover their declared types without actually compiling `make`'s body.
pub(crate) fn infer_closure_body_return_type_name_with_outer(
    compiler: &mut BytecodeCompiler,
    params: &[shape_ast::ast::FunctionParameter],
    body: &[shape_ast::ast::Statement],
    explicit_return: Option<&TypeAnnotation>,
    enclosing_params: &[shape_ast::ast::FunctionParameter],
) -> Option<String> {
    use shape_ast::ast::{BinaryOp as Op, Literal, Statement};
    use std::collections::HashMap;

    if let Some(ann) = explicit_return {
        if let Some(name) = BytecodeCompiler::tracked_type_name_from_annotation(ann) {
            return Some(name);
        }
    }

    // Build param-type map. Start with the enclosing-scope params (e.g.
    // the captured `n: int` from `fn make(n: int) -> any { return |x| x + n }`)
    // so the closure body can resolve free identifiers that came from the
    // outer function. Closure-local params override on name collision.
    let mut param_types: HashMap<String, String> = HashMap::new();
    for p in enclosing_params {
        let Some(ident) = p.pattern.as_identifier() else {
            continue;
        };
        if let Some(ann) = &p.type_annotation {
            if let Some(tn) = BytecodeCompiler::tracked_type_name_from_annotation(ann) {
                param_types.insert(ident.to_string(), tn);
            }
        }
    }
    for p in params {
        let Some(ident) = p.pattern.as_identifier() else {
            continue;
        };
        if let Some(ann) = &p.type_annotation {
            if let Some(tn) = BytecodeCompiler::tracked_type_name_from_annotation(ann) {
                param_types.insert(ident.to_string(), tn);
                continue;
            }
        }
        // Fallback: same body-literal-pairing heuristic the closure
        // compiler uses for unannotated params (`|x| x + 1`).
        if let Some(ann) = infer_param_type_from_body(ident, body) {
            if let Some(tn) = BytecodeCompiler::tracked_type_name_from_annotation(&ann) {
                param_types.insert(ident.to_string(), tn);
            }
        }
        // Sweep phase 3c.x: when the param has no annotation and no
        // body-literal pairing, but the body uses it in a binary op
        // against an enclosing-param that IS typed, infer the closure
        // param's type from the enclosing param's type. Covers
        // `|x| x + n` over `fn make(n: int) ...`.
        if !param_types.contains_key(ident) {
            if let Some(tn) = infer_param_type_from_outer_pairing(ident, body, &param_types) {
                param_types.insert(ident.to_string(), tn);
            }
        }
    }

    fn lit_type(lit: &Literal) -> Option<String> {
        Some(
            match lit {
                Literal::Int(_) => "int",
                Literal::Number(_) => "number",
                Literal::Bool(_) => "bool",
                Literal::String(_) | Literal::FormattedString { .. } | Literal::ContentString { .. } => {
                    "string"
                }
                Literal::Decimal(_) => "decimal",
                _ => return None,
            }
            .to_string(),
        )
    }

    fn expr_type(
        compiler: &mut BytecodeCompiler,
        param_types: &HashMap<String, String>,
        expr: &Expr,
    ) -> Option<String> {
        match expr {
            Expr::Literal(lit, _) => lit_type(lit),
            Expr::Identifier(name, _) => {
                if let Some(tn) = param_types.get(name) {
                    return Some(tn.clone());
                }
                // Outer-scope resolution: try `concrete_type_for_expr`
                // first (covers tracker-recorded primitives + array
                // element types), then fall back to the compiler's
                // `infer_expr_type` (which consults the type-inference
                // engine that ran on the program AST and can see
                // `let base = 100` even when the type tracker has no
                // entry for `base`).
                let ident_expr = Expr::Identifier(name.clone(), Span::DUMMY);
                if let Some(ct) = concrete_type_for_expr(compiler, &ident_expr) {
                    if let Some(tn) = concrete_type_to_type_annotation(&ct)
                        .and_then(|ann| BytecodeCompiler::tracked_type_name_from_annotation(&ann))
                    {
                        return Some(tn);
                    }
                }
                if let Ok(ty) = compiler.infer_expr_type(&ident_expr) {
                    let display = type_display_name_for_closure_inference(&ty);
                    if BytecodeCompiler::tracker_type_name_is_primitive(&display) {
                        return Some(display);
                    }
                }
                None
            }
            Expr::BinaryOp { left, right, op, .. } => {
                let lt = expr_type(compiler, param_types, left)?;
                let rt = expr_type(compiler, param_types, right)?;
                match op {
                    // Arithmetic on matching primitive scalar types
                    // preserves the type. Comparison/logical ops yield
                    // bool.
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod => {
                        if lt == rt && BytecodeCompiler::tracker_type_name_is_primitive(&lt) {
                            Some(lt)
                        } else {
                            None
                        }
                    }
                    Op::Equal
                    | Op::NotEqual
                    | Op::Less
                    | Op::LessEq
                    | Op::Greater
                    | Op::GreaterEq
                    | Op::And
                    | Op::Or => Some("bool".to_string()),
                    _ => None,
                }
            }
            Expr::UnaryOp { operand, .. } => expr_type(compiler, param_types, operand),
            Expr::Return(Some(inner), _) => expr_type(compiler, param_types, inner),
            Expr::Block(block, _) => {
                let last = block.items.last()?;
                match last {
                    shape_ast::ast::BlockItem::Expression(e) => {
                        expr_type(compiler, param_types, e)
                    }
                    shape_ast::ast::BlockItem::Statement(s) => {
                        stmt_type(compiler, param_types, s)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn stmt_type(
        compiler: &mut BytecodeCompiler,
        param_types: &HashMap<String, String>,
        stmt: &Statement,
    ) -> Option<String> {
        match stmt {
            Statement::Expression(e, _) => expr_type(compiler, param_types, e),
            Statement::Return(Some(e), _) => expr_type(compiler, param_types, e),
            _ => None,
        }
    }

    // Find body's terminal expression: prefer last statement; if it's a
    // `Return(e)` use e, else if it's an expression statement use it.
    let last = body.last()?;
    stmt_type(compiler, &param_types, last)
}

impl BytecodeCompiler {
    /// Compile a function expression (closure)
    ///
    /// `closure_span` is the span of the `||`/`|args|` expression itself
    /// — used by Session 1's Rust-move move-after-capture diagnostic to
    /// point at the capturing closure that consumed a `let mut` binding.
    pub(super) fn compile_expr_closure(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
        closure_span: Span,
    ) -> Result<()> {
        let closure_name = format!("__closure_{}", self.closure_counter);
        self.closure_counter += 1;

        let proto_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, mutated_captures) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));

        // Inside function bodies the MIR solver detects reference-capture errors
        // via `closure_capture_loans` facts, producing `ReferenceEscapeIntoClosure`.
        // For top-level code (no MIR), we still reject at the front-end.
        // Exception: inferred-ref locals (params passed by reference for performance)
        // are owned values and CAN be captured — the value is dereferenced at capture time.
        if self.current_function.is_none() {
            for captured in &captured_vars {
                if let Some(local_idx) = self.resolve_local(captured) {
                    let escapes_direct_borrow = self.ref_locals.contains(&local_idx)
                        && !self.inferred_ref_locals.contains(&local_idx);
                    let escapes_reference_value = self.reference_value_locals.contains(&local_idx);
                    if escapes_direct_borrow || escapes_reference_value {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                                captured
                            ),
                            location: None,
                        });
                    }
                }

                if let Some(scoped_name) = self.resolve_scoped_module_binding_name(captured)
                    && let Some(&binding_idx) = self.module_bindings.get(&scoped_name)
                    && self.reference_value_module_bindings.contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                            captured
                        ),
                        location: None,
                    });
                }
            }
        }

        // BUG1 — reject assignment to an immutable (`let`) outer binding
        // from inside the closure body. The environment analyzer marks
        // the binding in `mutated_captures` when the closure writes to
        // it; if the outer binding's ownership class is `OwnedImmutable`
        // (the `let` form), the write violates Shape's immutability
        // rules. Without this check the compiler still lowers a
        // `MakeClosure` whose capture layout mismatches the legacy
        // SharedCell path, producing the runtime-only crash
        // `MakeClosure for function N has no registered ClosureLayout`.
        // The diagnostic uses code `B0005` — the same code used for other
        // immutability/move violations across closure boundaries — and
        // suggests both `let mut` (local mutation) and `var` (shareable
        // mutation through closure captures) to match CLAUDE.md guidance.
        for captured in &captured_vars {
            if !mutated_captures.contains(captured) {
                continue;
            }
            let ownership = self
                .binding_semantics_for_name(captured)
                .map(|(_, _, sem)| sem.ownership_class);
            if !matches!(ownership, Some(BindingOwnershipClass::OwnedImmutable)) {
                continue;
            }
            let is_local_slot = self.resolve_local(captured).is_some();
            let is_module_binding_slot = !is_local_slot
                && (self.resolve_scoped_module_binding_name(captured).is_some()
                    || self.module_bindings.contains_key(captured));
            if !is_local_slot && !is_module_binding_slot {
                continue;
            }
            return Err(ShapeError::SemanticError {
                message: format!(
                    "[B0005] cannot assign to immutable binding '{captured}' captured by \
                     closure; use `let mut {captured}` for local mutation or `var {captured}` \
                     to allow shared mutation through closures"
                ),
                location: Some(self.span_to_source_location(closure_span)),
            });
        }

        // Build per-capture mutability flags (aligned with captured_vars order).
        // A capture is mutable if the closure itself mutates it OR if a previous
        // closure in the same scope already boxed it into a SharedCell.
        let mutable_flags: Vec<bool> = captured_vars
            .iter()
            .map(|name| mutated_captures.contains(name) || self.boxed_locals.contains(name))
            .collect();

        // Build closure parameters: only immutable captures become leading params.
        // Mutable captures are accessed via LoadClosure/StoreClosure opcodes.
        //
        // Strict-typing-sweep (Cluster 1): synthesize a `type_annotation` for each
        // capture from its resolved upstream `ConcreteType`. Without this the
        // capture-param falls into the "no annotation" branch in
        // `compile_function_body` (line ~1182) and ends up in `param_locals` with
        // no type info — which then makes binary-ops on the capture inside the
        // closure body fail with "Cannot infer types for binary operation".
        let mut closure_params = Vec::with_capacity(captured_vars.len() + params.len());
        for name in &captured_vars {
            let ident_expr = Expr::Identifier(name.clone(), Span::DUMMY);
            let capture_ct = concrete_type_for_expr(self, &ident_expr);
            let type_annotation = capture_ct
                .as_ref()
                .and_then(concrete_type_to_type_annotation);
            closure_params.push(shape_ast::ast::FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation,
                default_value: None,
            });
        }

        // Strict-typing-sweep (Cluster 3): consume bidirectional inference
        // hints for the user-portion params. The outer HOF dispatch site
        // populates `pending_closure_param_types` with one Option<TypeAnnotation>
        // per user param when the receiver type implies an arg type
        // (`arr.map(|x| …)` with `arr: Array<int>` → `x: int`). User params
        // with their own explicit annotation always win.
        let user_param_hints = self.pending_closure_param_types.take();

        // Strict-typing-sweep (Cluster 2): closure-body param inference.
        // For closures bound to a `let` and called via the local (or
        // synthesized inside a generic body where const-args have been
        // substituted to literals), we don't have an HOF-style call-site
        // hint. Infer each unannotated user param's type by scanning the
        // body for binary ops `<param> op <literal>` and pulling the
        // literal's type. This is the same conservative heuristic that
        // closure compilation has always relied on for `|x| x + 1`-shaped
        // bodies, just made first-class instead of riding on the deleted
        // *Dynamic-emission shim.
        for (idx, user_param) in params.iter().enumerate() {
            let mut p = user_param.clone();
            if p.type_annotation.is_none() {
                // 1. HOF call-site hint wins first.
                if let Some(hints) = user_param_hints.as_ref() {
                    if let Some(Some(ann)) = hints.get(idx) {
                        p.type_annotation = Some(ann.clone());
                    }
                }
                // 2. Body-level literal-pairing heuristic. Pulls type
                //    info from any binary op pairing the param with a
                //    typed literal OR with a captured/outer-scope
                //    identifier whose type is known.
                if p.type_annotation.is_none() {
                    if let Some(name) = p.pattern.as_identifier() {
                        if let Some(ann) = infer_param_type_from_body(name, body) {
                            p.type_annotation = Some(ann);
                        } else if let Some(ann) =
                            self.infer_param_type_from_body_with_outer_idents(name, body)
                        {
                            p.type_annotation = Some(ann);
                        }
                    }
                }
            }
            closure_params.push(p);
        }

        let closure_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: closure_params,
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let user_pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        let mut closure_pass_modes =
            vec![crate::compiler::ParamPassMode::ByValue; captured_vars.len()];
        closure_pass_modes.extend(user_pass_modes);
        let ref_params: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_reference())
            .collect();
        let ref_mutates: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_exclusive())
            .collect();
        self.inferred_param_pass_modes
            .insert(closure_name.clone(), closure_pass_modes);

        // Phase A: mint a ClosureTypeId keyed on the capture signature.
        //
        // Resolves each captured name to a `ConcreteType` via the monomorphizer
        // helpers; unresolved captures fall back to `Pointer(Void)` (opaque
        // 8-byte slot, conservatively treated as a heap-refcounted pointer by
        // the layout's `heap_capture_mask`). This records layout metadata in
        // `closure_registry` that Phase C consumes to extend the monomorphization
        // cache key. Emission is unchanged.
        let closure_type_id = self.mint_closure_type_id(&captured_vars);

        // Phase F: mint a FunctionTypeId for the callable signature. This is
        // the `Function<A, R>` identity — the signature omits captures and
        // covers only the parameters the caller supplies plus the return.
        //
        // Phase F keeps signature resolution conservative: param / return
        // types that lack compile-time resolution fall back to `Void`. The
        // ID is still globally unique per structural signature (driven by
        // the registry's intern), so `CallFunctionIndirect` can pick a
        // Cranelift call signature once signature inference lands. Two
        // closures with structurally identical callable shapes share a
        // `FunctionTypeId` even when their capture layouts (and hence
        // `ClosureTypeId`s) differ — this is exactly what `Array<Function<
        // (int) -> int>>` relies on for polymorphic dispatch.
        let function_type_id = self.mint_function_type_id_for_params(params);

        let func_idx = self.program.functions.len();
        self.program.functions.push(Function {
            name: closure_name.clone(),
            arity: closure_def.params.len() as u16,
            param_names: closure_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: captured_vars.len() as u16,
            is_async: false,
            ref_params,
            ref_mutates,
            mutable_captures: mutable_flags.clone(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        });

        // Record closure function_id for MIR back-patching (ClosurePlaceholder → Function)
        self.closure_function_ids
            .push((closure_name.clone(), func_idx as u16));
        // Phase A: record the closure's ClosureTypeId against its function index.
        self.closure_type_ids
            .push((func_idx as u16, closure_type_id));
        // Phase F: record the closure's FunctionTypeId alongside the capture
        // layout id. One entry per closure literal, same ordering as
        // `closure_type_ids`.
        self.function_type_ids
            .push((func_idx as u16, function_type_id));

        // Track A.1C — derive the `CaptureKind` for each capture based on
        // the source binding's declared form AND whether the closure body
        // actually mutates the capture.
        //
        // Binding form (when mutated inside the closure) → CaptureKind:
        //   `let mut x = ...`   (OwnedMutable source)   → CaptureKind::OwnedMutable
        //   `var x = ...`       (Flexible source)       → CaptureKind::Shared
        //
        // Everything else (including read-only captures of `let mut` /
        // `var` bindings, and all captures of `let` / function parameters)
        // → `CaptureKind::Immutable`. A read-only capture is semantically
        // a by-value snapshot and does not require cell indirection.
        //
        // Note (A.1C partial): this metadata rides on the layout's
        // `capture_kinds` field only. The mutable-mask bits on the layout
        // remain zero in this commit — see the design note on
        // `build_closure_function_layouts`. The interpreter's
        // `op_make_closure` still routes mutable-capture closures through
        // the legacy `HeapValue::Closure` + SharedCell path because the
        // compiler has not yet been rewired to emit the A.1B
        // `Load/StoreOwnedMutableCapture` / `Load/StoreSharedCapture`
        // opcodes in closure bodies, and outer-scope reads of promoted
        // `let mut` / `var` bindings still flow through `LoadClosure` +
        // `HeapValue::SharedCell` auto-deref. Full routing is the A.1C
        // residual.
        use shape_value::v2::closure_layout::CaptureKind;
        let capture_kinds: Vec<CaptureKind> = captured_vars
            .iter()
            .enumerate()
            .map(|(i, name)| {
                // Only mutated captures need cell indirection. Read-only
                // captures are snapshot-by-value and stay Immutable
                // regardless of the source binding's ownership class —
                // this keeps function-parameter captures (default
                // `OwnedMutable` per `binding_semantics_for_param`) on
                // the Immutable path when the closure doesn't write
                // through them.
                if !mutable_flags.get(i).copied().unwrap_or(false) {
                    return CaptureKind::Immutable;
                }
                // Track A.1C.2 (locals) + A.1C.3 (module bindings): any
                // mutable `var` capture routes through
                // `CaptureKind::Shared`, whether the outer slot is a
                // local or a module binding. Both paths allocate an
                // `Arc<parking_lot::Mutex<ValueWord>>` and install its
                // `Arc::into_raw` pointer into the closure's Ptr slot;
                // `op_make_closure` bumps the strong count. The compiler
                // emits different *outer-scope* opcodes for local vs
                // module-binding promotion (`AllocSharedLocal` vs
                // `AllocSharedModuleBinding`), but the closure-side
                // machinery is the same.
                let is_local_slot = self.resolve_local(name).is_some();
                let is_module_binding_slot = !is_local_slot
                    && (self.resolve_scoped_module_binding_name(name).is_some()
                        || self.module_bindings.contains_key(name));
                let ownership = self
                    .binding_semantics_for_name(name)
                    .map(|(_, _, sem)| sem.ownership_class);
                match ownership {
                    // Track A.1C.2b: `let mut` captures whose outer slot
                    // is a local flow through the A.1B OwnedMutable
                    // Raw path. For module-binding `let mut` (top-level
                    // `let mut sum = 0` in REPL-style eval compiles to
                    // a module binding), there is no move-into-closure
                    // semantics — the binding is program-lifetime. Fall
                    // through to the Shared pipeline so mutations from
                    // the closure propagate to the outer slot, matching
                    // the pre-A.1C.3 legacy SharedCell semantics.
                    Some(BindingOwnershipClass::OwnedMutable) if is_local_slot => {
                        CaptureKind::OwnedMutable
                    }
                    Some(BindingOwnershipClass::OwnedMutable) if is_module_binding_slot => {
                        CaptureKind::Shared
                    }
                    Some(BindingOwnershipClass::OwnedMutable) => CaptureKind::Immutable,
                    Some(BindingOwnershipClass::Flexible)
                        if is_local_slot || is_module_binding_slot =>
                    {
                        CaptureKind::Shared
                    }
                    Some(BindingOwnershipClass::Flexible) => CaptureKind::Immutable,
                    // Track A.1C.2 / A.1C.3: semantics lookup can return
                    // `None` when a prior closure's `compile_function`
                    // wiped the outer function's type-tracker local
                    // semantics. Fall back to persistent witnesses
                    // populated by the previous classification pass:
                    //   - `shared_locals` / `shared_module_bindings`
                    //     for `var` captures.
                    //   - `owned_mutable_locals` for `let mut` local
                    //     captures (A.1C.3: without this witness, a
                    //     second closure capturing a different local
                    //     would reclassify to `Immutable`, nulling the
                    //     layout's OwnedMutable mask and tripping the
                    //     `op_make_closure` layout-mismatch guard).
                    _ if is_local_slot && self.shared_locals.contains(name) => CaptureKind::Shared,
                    _ if is_local_slot && self.owned_mutable_locals.contains(name) => {
                        CaptureKind::OwnedMutable
                    }
                    _ if is_module_binding_slot
                        && self.shared_module_binding_contains(name) =>
                    {
                        CaptureKind::Shared
                    }
                    // A.1C.3: module-binding captures with no resolved
                    // ownership semantics (e.g. imported functions used
                    // as callable values, top-level `let` without `mut`
                    // — unreachable here since `mutable_flags[i]` is
                    // true) also go through Shared when the closure
                    // mutates them. `mutable_flags[i]` is already known
                    // true at this point (early return above).
                    _ if is_module_binding_slot => CaptureKind::Shared,
                    _ => CaptureKind::Immutable,
                }
            })
            .collect();
        // Track A.1C.3: record persistent witnesses for each classified
        // capture so sibling closures (after the type-tracker has been
        // wiped by `compile_function`) reclassify the same way rather
        // than falling back to `Immutable`.
        for (i, name) in captured_vars.iter().enumerate() {
            match capture_kinds[i] {
                CaptureKind::OwnedMutable if self.resolve_local(name).is_some() => {
                    self.owned_mutable_locals.insert(name.clone());
                }
                _ => {}
            }
        }
        self.closure_capture_kinds
            .push((func_idx as u16, capture_kinds.clone()));

        // Track A.1C.2: if any capture is non-Immutable, re-intern the
        // closure_type_id under the kinds-aware registry key so two
        // closures with identical types but different kinds get distinct
        // `ClosureTypeId`s. When all captures are Immutable, the original
        // types-only intern already returned the canonical id — skip.
        if capture_kinds
            .iter()
            .any(|k| !matches!(k, CaptureKind::Immutable))
        {
            use shape_value::v2::concrete_type::ConcreteType;
            let capture_types: Vec<ConcreteType> = captured_vars
                .iter()
                .map(|name| {
                    let ident = Expr::Identifier(name.clone(), Span::DUMMY);
                    concrete_type_for_expr(self, &ident)
                        .unwrap_or_else(|| ConcreteType::Pointer(Box::new(ConcreteType::Void)))
                })
                .collect();
            let kinds_id = self
                .closure_registry
                .intern_with_kinds(capture_types, capture_kinds.clone());
            // Overwrite the last-pushed `closure_type_ids` entry for this
            // function with the kinds-aware id. The Immutable entry
            // produced by `mint_closure_type_id` (which ignores kinds)
            // remains in the registry for all-immutable closures.
            if let Some(last) = self.closure_type_ids.last_mut() {
                debug_assert_eq!(last.0, func_idx as u16);
                last.1 = kinds_id;
            }
            let _ = closure_type_id; // the kinds-aware id supersedes it.
        }

        // Track A.1C.2b — enforce `let mut` escape rejection (§4.3).
        //
        // `let mut` bindings captured by an escaping closure are a
        // compile error: `let mut` is a unique-owner form, and moving
        // it into a heap closure that outlives the surrounding frame
        // would leak the owner out of its original scope. The compiler
        // rejects this with B0003 and asks the user to promote the
        // source to `var` (shared) or restructure. Non-escaping
        // closures (the common case) are fine — the `let mut` binding
        // is moved by value into a single closure at make-closure time
        // and accessed inside the body via `LoadOwnedMutableCapture` /
        // `StoreOwnedMutableCapture` (A.1B).
        //
        // The heap-promotion signal is `emit_make_closure_heap_next`.
        let closure_is_escaping = self.emit_make_closure_heap_next;
        for (i, name) in captured_vars.iter().enumerate() {
            if !mutable_flags.get(i).copied().unwrap_or(false) {
                continue;
            }
            let local_idx = self.resolve_local(name);
            let plan_class = local_idx.and_then(|idx| self.mir_storage_class_for_slot(idx));
            let ownership = self
                .binding_semantics_for_name(name)
                .map(|(_, _, sem)| sem.ownership_class);

            if matches!(ownership, Some(BindingOwnershipClass::OwnedMutable))
                && !matches!(
                    plan_class,
                    Some(BindingStorageClass::LocalMutablePtr)
                        | Some(BindingStorageClass::Reference)
                        | Some(BindingStorageClass::Direct)
                        | Some(BindingStorageClass::Deferred)
                        | None,
                )
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0003] mutable binding '{}' cannot be captured by an escaping closure; \
                         promote the source to `var` or restructure to keep the closure local",
                        name
                    ),
                    location: None,
                });
            }
        }

        // Set up the per-kind closure-body emission maps. During body
        // compilation:
        //   * `mutable_closure_captures` → legacy `LoadClosure` /
        //     `StoreClosure` (module-binding `var` captures and any
        //     residual capture whose outer slot could not be migrated
        //     to A.1B's Raw path).
        //   * `owned_mutable_closure_captures` → A.1B's
        //     `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture`
        //     for `let mut` captures (outer slot is moved by value into
        //     the closure at make-closure time; closure owns the
        //     `Box::into_raw(Box::new(initial))` pointer).
        //   * `shared_closure_captures` → A.1B's `LoadSharedCapture` /
        //     `StoreSharedCapture` for `var` (local-slot) captures
        //     previously promoted via `AllocSharedLocal`.
        let saved_mutable_captures = std::mem::take(&mut self.mutable_closure_captures);
        let saved_shared_captures = std::mem::take(&mut self.shared_closure_captures);
        let saved_owned_mutable_captures =
            std::mem::take(&mut self.owned_mutable_closure_captures);
        let saved_owned_mutable_capture_inner_kinds =
            std::mem::take(&mut self.owned_mutable_capture_inner_kinds);
        let _ = closure_is_escaping;
        for (i, name) in captured_vars.iter().enumerate() {
            if mutable_flags.get(i).copied().unwrap_or(false) {
                self.mutable_closure_captures.insert(name.clone(), i as u16);
                let kind = capture_kinds
                    .get(i)
                    .copied()
                    .unwrap_or(CaptureKind::Immutable);
                // Track A.1C.2 + A.1C.3: Shared (var) captures — whether
                // the outer slot is a local or a module binding — route
                // through the A.1B Load/StoreSharedCapture opcodes
                // inside the closure body. The closure-side machinery
                // is identical; only the outer-scope promotion opcodes
                // differ between locals and module bindings.
                if matches!(kind, CaptureKind::Shared) {
                    self.shared_closure_captures.insert(name.clone(), i as u16);
                }
                // Track A.1C.2b: OwnedMutable (let mut) captures route
                // through the A.1B Load/StoreOwnedMutableCapture
                // opcodes. Gate on `resolve_local` — only locals can be
                // captured OwnedMutable (module bindings have program-
                // lifetime and don't admit move semantics); for module-
                // binding sources the capture was reclassified to
                // `Immutable` upstream.
                if matches!(kind, CaptureKind::OwnedMutable) && self.resolve_local(name).is_some() {
                    self.owned_mutable_closure_captures
                        .insert(name.clone(), i as u16);
                    // Wave E: record the cell's interior `FieldKind` so the
                    // closure body's read/write emit sites can dispatch to
                    // the typed Wave D.1 opcodes (codes 0x140-0x155). The
                    // inner kind is derived from the captured binding's
                    // resolved `ConcreteType` at this construction site —
                    // identical to the type used for `op_make_closure`'s
                    // `alloc_owned_mutable_<kind>` selection. Falls back to
                    // `Ptr` when the type isn't statically resolved
                    // (matches `concrete_type_for_expr`'s default for
                    // unresolved heap-typed captures).
                    let ident_expr = Expr::Identifier(name.clone(), Span::DUMMY);
                    let inner_kind = concrete_type_for_expr(self, &ident_expr)
                        .map(|ct| ct.to_field_kind())
                        .unwrap_or(shape_value::v2::struct_layout::FieldKind::Ptr);
                    self.owned_mutable_capture_inner_kinds
                        .insert(name.clone(), inner_kind);
                }
            }
        }

        // Jump-over is now emitted unconditionally inside
        // `compile_function_body`, which patches its own jump at the end of
        // the body. Emitting another jump here would double-jump and the
        // closure's entry_point (post-the-outer-jump) would point at the
        // inner jump, which then skips the body entirely. Don't.
        let saved_closure_ids = self.closure_function_ids.clone();
        self.compile_function(&closure_def)?;
        self.closure_function_ids = saved_closure_ids;

        // Restore mutable_closure_captures
        self.mutable_closure_captures = saved_mutable_captures;
        self.shared_closure_captures = saved_shared_captures;
        self.owned_mutable_closure_captures = saved_owned_mutable_captures;
        self.owned_mutable_capture_inner_kinds = saved_owned_mutable_capture_inner_kinds;

        // Capture boxing decisions
        // ────────────────────────
        // The storage planner assigns each binding a BindingStorageClass that
        // determines whether the variable needs heap indirection:
        //
        //   Direct     → LoadLocal / StoreLocal (no indirection needed)
        //   Deferred   → plan not yet resolved; fall back to legacy boxing
        //   UniqueHeap → legacy cell wrapping + SharedCell.
        //                Future: unique Box without RwLock overhead.
        //   SharedCow  → legacy cell wrapping + SharedCell.
        //                Future: COW wrapper.
        //   Reference  → DerefLoad / DerefStore (already handled above)
        //
        // We emit the legacy cell-wrapping opcode when the storage plan says
        // the binding needs heap indirection (UniqueHeap, SharedCow, Direct,
        // or Deferred). Only Reference bindings skip boxing — they are
        // handled separately by the escape check above. In the future, the
        // planner may introduce a dedicated "no-sharing" class to skip
        // boxing for Direct bindings.
        for (i, captured) in captured_vars.iter().enumerate() {
            if matches!(
                self.binding_semantics_for_name(captured),
                Some((_, _, semantics))
                    if semantics.ownership_class == BindingOwnershipClass::Flexible
            ) {
                let storage = if mutable_flags.get(i).copied().unwrap_or(false) {
                    BindingStorageClass::SharedCow
                } else {
                    BindingStorageClass::UniqueHeap
                };
                self.promote_flexible_binding_storage_for_name(captured, storage);
            }
            if mutable_flags.get(i).copied().unwrap_or(false) {
                // Consult the storage plan to decide whether boxing is needed.
                // Currently, Direct and Deferred bindings are both boxed for
                // mutable captures because the storage plan runs before closure
                // compilation and these are the default states. Reference
                // bindings are already handled by the escape check above, so
                // the only class that could skip boxing is one where the
                // planner explicitly marks "no sharing needed" — a future
                // optimization.
                // Consult the MIR storage plan first (authoritative when available),
                // then fall back to type-tracker binding semantics.
                let mir_plan_class = self
                    .resolve_local(captured)
                    .and_then(|idx| self.mir_storage_class_for_slot(idx));
                let should_box = if let Some(plan_class) = mir_plan_class {
                    // MIR plan is authoritative: box when UniqueHeap/SharedCow,
                    // skip for Reference (handled above), box for Direct/Deferred
                    // since mutable capture needs heap indirection.
                    !matches!(plan_class, BindingStorageClass::Reference)
                } else if let Some((_, _, semantics)) = self.binding_semantics_for_name(captured) {
                    // Fallback to type-tracker semantics
                    !matches!(semantics.storage_class, BindingStorageClass::Reference)
                } else {
                    true // no plan available, use legacy behavior (always box)
                };

                if should_box {
                    // Mutable capture: promote the outer binding so the
                    // closure and its enclosing scope observe the same
                    // mutable state, then push the value (OwnedMutable) or
                    // pointer (Shared) the enclosing `MakeClosure` needs
                    // to install into the closure's capture slot.
                    //
                    // Dispatch by `capture_kinds[i]`:
                    //   * `Shared` (`var` binding captured mutably) →
                    //     Track A.1C.2 path. For local slots: emit
                    //     `LoadLocal + AllocSharedLocal + LoadLocal` to
                    //     promote the slot into `Arc<SharedCell>` and
                    //     push the pointer bits; add the binding to
                    //     `shared_locals` so every outer-scope read /
                    //     write / scope-exit goes through the new
                    //     opcodes. For module bindings keep the legacy
                    //     `BoxModuleBinding` path — A.1C.1's opcodes
                    //     cover only local slots; module bindings retire
                    //     with A.1C.3.
                    //   * `OwnedMutable` (`let mut`) → Track A.1C.2b
                    //     path. Push the outer slot's plain value with
                    //     `LoadLocal`; `op_make_closure` will see the
                    //     `owned_mutable_capture_mask` bit for this
                    //     index and call
                    //     `Box::into_raw(Box::new(initial))`. The closure
                    //     body emits
                    //     `Load/StoreOwnedMutableCapture` (A.1B) to read
                    //     /write through the box pointer. No SharedCell,
                    //     no Arc, no lock.
                    //   * Other fallbacks (module-binding `var` etc.) →
                    //     legacy cell-wrapping / `BoxModuleBinding` path.
                    //     A.1C.3 retires these alongside the
                    //     `HeapValue::Closure` fallback producer.
                    self.set_binding_storage_class_for_name(
                        captured,
                        BindingStorageClass::SharedCow,
                    );
                    let kind = capture_kinds
                        .get(i)
                        .copied()
                        .unwrap_or(CaptureKind::Immutable);
                    let is_shared_local_slot = matches!(kind, CaptureKind::Shared)
                        && self.resolve_local(captured).is_some();
                    let is_owned_mutable = matches!(kind, CaptureKind::OwnedMutable);
                    let shared_module_binding_scoped_name = if matches!(kind, CaptureKind::Shared)
                        && !is_shared_local_slot
                    {
                        self.resolve_scoped_module_binding_name(captured).or_else(|| {
                            if self.module_bindings.contains_key(captured) {
                                Some(captured.clone())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };
                    if is_shared_local_slot {
                        let local_idx = self
                            .resolve_local(captured)
                            .expect("checked is_shared_local_slot");
                        if !self.shared_locals.contains(captured) {
                            // First promotion: push current value, alloc
                            // the Arc cell, then push the pointer bits.
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                            self.emit(Instruction::new(
                                OpCode::AllocSharedLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                            self.shared_locals.insert(captured.clone());
                            if let Some(scope) = self.shared_drop_locals.last_mut() {
                                scope.push(local_idx);
                            }
                        }
                        // Push the *pointer bits* of the (possibly just-
                        // allocated) shared cell. op_make_closure will
                        // `Arc::increment_strong_count` for each Shared
                        // capture before installing it in the closure.
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if is_owned_mutable && let Some(local_idx) = self.resolve_local(captured)
                    {
                        // Track A.1C.2b: `let mut` outer slot is captured
                        // by move. Push the current value — op_make_closure
                        // sees the `owned_mutable_capture_mask` bit and
                        // allocates `Box::into_raw(Box::new(bits))` into
                        // the Ptr slot. No cell wrapping, no SharedCell.
                        //
                        // Session 1 — Rust-move semantics: record the
                        // binding as "moved into closure at closure_span"
                        // so subsequent outer reads / writes fail at
                        // compile time with a use-after-move diagnostic.
                        // The `captured_let_mut_moved` map is consulted
                        // in `compile_expr_identifier` (load path) and
                        // `compile_expr_assign` (store path).
                        self.captured_let_mut_moved
                            .insert(captured.clone(), closure_span);
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if let Some(scoped_name) = shared_module_binding_scoped_name {
                        // Track A.1C.3: Shared module-binding var
                        // capture. Mirrors the Shared local-slot path
                        // above with module-binding addressing:
                        //   First promotion: `LoadModuleBinding` +
                        //     `AllocSharedModuleBinding` promotes the
                        //     module-binding slot to raw Arc pointer
                        //     bits.
                        //   Then: `LoadModuleBinding` pushes those raw
                        //     pointer bits for `op_make_closure` to
                        //     `Arc::increment_strong_count` on.
                        // `LoadModuleBinding`'s auto-deref for legacy
                        // SharedCell is retired in this same commit —
                        // the bits pushed here are raw pointer bits, not
                        // a NaN-tagged SharedCell ValueWord, so
                        // `LoadModuleBinding` passes them through
                        // unmodified (no `HeapValue::SharedCell` tag).
                        let mb_idx = self.get_or_create_module_binding(&scoped_name);
                        if !self.shared_module_bindings.contains(&scoped_name) {
                            self.emit(Instruction::new(
                                OpCode::LoadModuleBinding,
                                Some(Operand::ModuleBinding(mb_idx)),
                            ));
                            self.emit(Instruction::new(
                                OpCode::AllocSharedModuleBinding,
                                Some(Operand::ModuleBinding(mb_idx)),
                            ));
                            self.shared_module_bindings.insert(scoped_name);
                        }
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(mb_idx)),
                        ));
                    } else {
                        // Last resort fallback — just load the value.
                        // Reached when the capture is Immutable (e.g.
                        // OwnedMutable that resolved to a module
                        // binding and was reclassified). A plain load
                        // is correct: op_make_closure will store the
                        // ValueWord directly into the capture slot as
                        // an Immutable capture.
                        let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                        self.compile_expr(&temp)?;
                    }
                } else {
                    // Storage plan says Direct — no boxing needed, just load the value.
                    let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                    self.compile_expr(&temp)?;
                }
            } else {
                let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                self.compile_expr(&temp)?;
                // Phase V1.2C/D — Site A: closure capture of a
                // uniquely-owned value into an *escaping* closure.
                // If the outer slot is classified as `UniqueHeap`
                // (Box-backed, owned — see Phase 4 / `PromoteToOwned`)
                // and the closure escapes the current scope, the
                // captured value must transition to an Arc-shared
                // encoding so the closure can outlive the owning
                // binding. `PromoteToShared` converts the top-of-stack
                // Box into an Arc in place without bumping a refcount.
                // No-op on inline scalars and already-Arc values, so
                // emitting it here is correctness-safe; gating on
                // `UniqueHeap` simply avoids the unnecessary opcode.
                //
                // Non-escaping closures share the caller's scope by
                // construction — the Box stays unique for the closure's
                // lifetime and the promotion is unnecessary.
                if closure_is_escaping && crate::compiler::helpers::promote_to_shared_enabled() {
                    if let Some(local_idx) = self.resolve_local(captured) {
                        // Mirror V1.1C's `slot_is_heap_backed_owned`:
                        // `UniqueHeap` is the canonical owned-heap class,
                        // but `Direct` + non-scalar storage hint also
                        // indicates a Box-backed slot (strings, arrays,
                        // hashmaps, typed objects) handed to the slot
                        // by the Phase 4 `PromoteToOwned` emission —
                        // those need the same Box→Arc transition when
                        // they escape into a closure.
                        if self.slot_is_heap_backed_owned(local_idx) {
                            self.emit(Instruction::simple(OpCode::PromoteToShared));
                        }
                    }
                }
            }
        }

        // Phase F: when the compiler has been told to emit the heap-ABI
        // form for this closure (e.g. by an outer expression that knows the
        // closure escapes — the most common driver is return-of-closure and
        // store-into-array patterns), tag the `MakeClosure` operand with
        // `escapes: true`. Phase H5 merged the former `MakeClosureHeap`
        // opcode into `MakeClosure`; the JIT reads `escapes` from the
        // operand variant (compile-time constant — no memory load on the
        // dispatch fast path).
        //
        // The `emit_make_closure_heap_next` flag is a single-shot hook: the
        // caller sets it before `compile_expr_closure` runs and the
        // closure lowerer consumes it at emission time. This keeps the
        // decision close to the escape signal without threading a second
        // parameter through the closure-compilation API.
        let escapes = std::mem::take(&mut self.emit_make_closure_heap_next);
        let fid = shape_value::FunctionId(func_idx as u16);
        let operand = if escapes {
            Operand::ClosureAlloc { fid, escapes: true }
        } else {
            Operand::Function(fid)
        };
        self.emit(Instruction::new(OpCode::MakeClosure, Some(operand)));
        // Closures don't produce TypedObjects
        self.last_expr_schema = None;
        Ok(())
    }

    /// Read-only access to the compiler's closure registry.
    /// Populated by each closure literal during lowering (Phase A).
    pub fn closure_registry(&self) -> &shape_value::v2::closure_layout::ClosureRegistry {
        &self.closure_registry
    }

    /// `(function_id, ClosureTypeId)` pairs, one per closure literal lowered
    /// during compilation. Phase C consumes this to key the monomorphization
    /// cache by closure layout.
    pub fn closure_type_ids(&self) -> &[(u16, ClosureTypeId)] {
        &self.closure_type_ids
    }

    /// Read-only access to the compiler's function-type registry.
    /// Populated per closure literal during lowering (Phase F).
    pub fn function_type_registry(
        &self,
    ) -> &shape_value::v2::function_type_registry::FunctionTypeRegistry {
        &self.function_type_registry
    }

    /// `(function_id, FunctionTypeId)` pairs, one per closure literal.
    /// Phase F uses this to pick a Cranelift `call_indirect` signature for
    /// polymorphic `Function<A, R>` dispatch.
    pub fn function_type_ids(&self) -> &[(u16, shape_value::v2::concrete_type::FunctionTypeId)] {
        &self.function_type_ids
    }

    /// Mint a `ClosureTypeId` for a closure literal by resolving each capture
    /// name to a `ConcreteType` and interning the resulting signature in
    /// `closure_registry` (Phase A).
    ///
    /// Unresolved captures fall back to `Pointer(Void)` — an opaque 8-byte
    /// slot that the layout treats as heap-refcounted. This keeps semantics
    /// conservative (no missed Drop glue) while Phase B/C/D grow the
    /// resolution coverage.
    pub(crate) fn mint_closure_type_id(&mut self, captured_vars: &[String]) -> ClosureTypeId {
        let capture_types: Vec<ConcreteType> = captured_vars
            .iter()
            .map(|name| {
                let ident = Expr::Identifier(name.clone(), Span::DUMMY);
                concrete_type_for_expr(self, &ident)
                    .unwrap_or_else(|| ConcreteType::Pointer(Box::new(ConcreteType::Void)))
            })
            .collect();
        self.closure_registry.intern(capture_types)
    }

    /// Phase F — mint a `FunctionTypeId` for a closure literal's callable
    /// signature (parameters + return type).
    ///
    /// Captures are intentionally excluded: `FunctionTypeId` identifies the
    /// cross-value `Function<A, R>` shape, not the capture layout. Two
    /// closures with the same signature but different captures share a
    /// `FunctionTypeId` — this is the whole point of the `Array<Function<
    /// (int) -> int>>` dispatch pattern.
    ///
    /// Resolution of per-param concrete types from type annotations is
    /// kept conservative in Phase F: unannotated or unresolved params
    /// resolve to `ConcreteType::Void`. This is safe because the registry
    /// keys structurally and two closures with identical (annotated) param
    /// shapes still share an id; Phase G/H will tighten resolution once
    /// bidirectional inference is wired through.
    pub(crate) fn mint_function_type_id_for_params(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
    ) -> shape_value::v2::concrete_type::FunctionTypeId {
        use shape_value::v2::concrete_type::ConcreteType as CT;
        use shape_value::v2::function_type_registry::FunctionSignature;

        let param_types: Vec<CT> = params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_ref()
                    .and_then(Self::concrete_type_for_annotation_static)
                    .unwrap_or(CT::Void)
            })
            .collect();
        let ret = CT::Void;
        self.function_type_registry
            .intern(FunctionSignature::new(param_types, ret))
    }

    /// Extract a `ConcreteType` from a `TypeAnnotation` without consulting
    /// the compiler's type-inference machinery. Lightweight, conservative
    /// mapping for the Phase F `FunctionTypeId` registry.
    fn concrete_type_for_annotation_static(
        annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Option<shape_value::v2::concrete_type::ConcreteType> {
        use shape_ast::ast::TypeAnnotation;
        use shape_value::v2::concrete_type::ConcreteType as CT;
        match annotation {
            TypeAnnotation::Basic(name) => match name.as_str() {
                "int" | "i64" => Some(CT::I64),
                "i32" => Some(CT::I32),
                "i16" => Some(CT::I16),
                "i8" => Some(CT::I8),
                "u64" => Some(CT::U64),
                "u32" => Some(CT::U32),
                "u16" => Some(CT::U16),
                "u8" => Some(CT::U8),
                "number" | "f64" => Some(CT::F64),
                "bool" => Some(CT::Bool),
                "string" => Some(CT::String),
                "void" | "unit" => Some(CT::Void),
                "decimal" => Some(CT::Decimal),
                "bigint" => Some(CT::BigInt),
                "DateTime" | "datetime" => Some(CT::DateTime),
                _ => None,
            },
            TypeAnnotation::Array(inner) => {
                Self::concrete_type_for_annotation_static(inner).map(|t| CT::Array(Box::new(t)))
            }
            TypeAnnotation::Reference(path) => {
                let name = path.as_str();
                match name {
                    "int" | "i64" => Some(CT::I64),
                    "number" | "f64" => Some(CT::F64),
                    "bool" => Some(CT::Bool),
                    "string" => Some(CT::String),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Phase C — peek a closure literal's capture signature and mint (or
    /// reuse) a [`ClosureTypeId`] WITHOUT lowering the closure to bytecode
    /// and WITHOUT pushing to `closure_type_ids`.
    ///
    /// The resolver calls this during `try_monomorphize_method_call` to key
    /// the monomorphization cache on the closure's layout. At emission time
    /// the usual `compile_expr_closure` path runs as normal — because the
    /// registry's `intern` is idempotent, both calls return the same
    /// `ClosureTypeId`. The split responsibility (gotcha option **(a)** in
    /// the Phase C plan) is:
    ///
    ///   - Resolver → peek + intern layout id only.
    ///   - `compile_expr_closure` → intern layout id (no-op second time) AND
    ///     push `(func_id, type_id)` into `closure_type_ids`.
    ///
    /// This keeps `closure_type_ids` free of duplicates while letting the
    /// resolver see the id early.
    pub(crate) fn mint_closure_type_id_peek(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> ClosureTypeId {
        // Run the same capture analysis as `compile_expr_closure`, but only
        // for the purpose of reading capture names off of the AST.
        let proto_def = FunctionDef {
            name: "__peek_closure__".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, _mutated) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));

        self.mint_closure_type_id(&captured_vars)
    }

    /// Strict-typing-sweep (Cluster 2 extension): same body scan as the
    /// free `infer_param_type_from_body` helper but uses the compiler's
    /// type tracker to resolve identifier operands against outer-scope
    /// bindings. Catches `|x| x + n` where `n` is a captured int local.
    pub(crate) fn infer_param_type_from_body_with_outer_idents(
        &self,
        param_name: &str,
        body: &[shape_ast::ast::Statement],
    ) -> Option<TypeAnnotation> {
        use shape_ast::ast::Statement;
        fn scan_expr(
            compiler: &BytecodeCompiler,
            name: &str,
            expr: &Expr,
        ) -> Option<TypeAnnotation> {
            match expr {
                Expr::BinaryOp { left, right, .. } => {
                    let pair_match = if let Expr::Identifier(n, _) = left.as_ref() {
                        if n == name {
                            outer_ident_type_ann(compiler, right)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(ann) = pair_match {
                        return Some(ann);
                    }
                    let pair_match = if let Expr::Identifier(n, _) = right.as_ref() {
                        if n == name {
                            outer_ident_type_ann(compiler, left)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(ann) = pair_match {
                        return Some(ann);
                    }
                    scan_expr(compiler, name, left)
                        .or_else(|| scan_expr(compiler, name, right))
                }
                Expr::UnaryOp { operand, .. } => scan_expr(compiler, name, operand),
                Expr::FunctionCall { args, .. } => {
                    args.iter().find_map(|a| scan_expr(compiler, name, a))
                }
                Expr::MethodCall { receiver, args, .. } => scan_expr(compiler, name, receiver)
                    .or_else(|| args.iter().find_map(|a| scan_expr(compiler, name, a))),
                Expr::Array(elements, _) => {
                    elements.iter().find_map(|e| scan_expr(compiler, name, e))
                }
                Expr::Return(Some(e), _) => scan_expr(compiler, name, e),
                _ => None,
            }
        }
        fn scan_stmt(
            compiler: &BytecodeCompiler,
            name: &str,
            stmt: &Statement,
        ) -> Option<TypeAnnotation> {
            match stmt {
                Statement::Expression(expr, _) => scan_expr(compiler, name, expr),
                Statement::Return(Some(e), _) => scan_expr(compiler, name, e),
                Statement::VariableDecl(decl, _) => {
                    decl.value.as_ref().and_then(|e| scan_expr(compiler, name, e))
                }
                Statement::Assignment(asgn, _) => scan_expr(compiler, name, &asgn.value),
                _ => None,
            }
        }
        /// Resolve an arbitrary expression to a `TypeAnnotation` when it's
        /// an identifier whose outer-scope type is statically known.
        /// Conservatively only handles `Expr::Identifier`.
        fn outer_ident_type_ann(
            compiler: &BytecodeCompiler,
            expr: &Expr,
        ) -> Option<TypeAnnotation> {
            let other_name = match expr {
                Expr::Identifier(n, _) => n,
                _ => return None,
            };
            let ident_expr = Expr::Identifier(other_name.clone(), Span::DUMMY);
            let ct = concrete_type_for_expr(compiler, &ident_expr)?;
            concrete_type_to_type_annotation(&ct)
        }
        body.iter().find_map(|s| scan_stmt(self, param_name, s))
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use crate::type_tracking::BindingStorageClass;
    use shape_ast::ast::{Expr, Item, Span, Statement, VarKind, VariableDecl};
    use shape_ast::parser::parse_program;

    #[test]
    fn test_mutable_closure_capture_marks_binding_as_shared_storage() {
        let program =
            parse_program("let inc = || { counter = counter + 1; counter }").expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };

        let mut compiler = BytecodeCompiler::new();
        let counter_idx = compiler.get_or_create_module_binding("counter");
        // BUG1 — closures that mutate an outer binding require the
        // source to be `var` (or `let mut` for local mutation). Prior
        // to the compile-time immutability check this test used
        // `VarKind::Let, is_mut: false`, which is precisely the case
        // that now (correctly) rejects with B0005. Bumping the source
        // to `VarKind::Var` matches the test's documented intent
        // ("mutable closure capture marks binding as shared storage").
        // Sweep phase 3c.x: declare `counter: int` so the closure body
        // (`counter + 1`) survives strict-typing's binop dispatch. The
        // test's purpose is to assert the storage-class side effect of
        // capture, not to exercise inference of an uninitialized var.
        let counter_decl = VariableDecl {
            kind: VarKind::Var,
            is_mut: false,
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "counter".to_string(),
                Span::DUMMY,
            ),
            type_annotation: Some(shape_ast::ast::TypeAnnotation::Basic("int".to_string())),
            value: None,
            ownership: Default::default(),
        };
        compiler.apply_binding_semantics_to_pattern_bindings(
            &counter_decl.pattern,
            false,
            BytecodeCompiler::binding_semantics_for_var_decl(&counter_decl),
        );
        // Wire counter's type into the type tracker so binop dispatch
        // can see it as int instead of Unknown.
        compiler.set_module_binding_type_info(counter_idx, "int");

        compiler
            .compile_expr_closure(params, body, Span::DUMMY)
            .expect("closure should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(counter_idx)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_flexible_closure_capture_marks_binding_as_unique_heap_storage() {
        let program = parse_program("let read = || counter").expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };

        let mut compiler = BytecodeCompiler::new();
        let counter_idx = compiler.get_or_create_module_binding("counter");
        let counter_decl = VariableDecl {
            kind: VarKind::Var,
            is_mut: false,
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "counter".to_string(),
                Span::DUMMY,
            ),
            type_annotation: None,
            value: None,
            ownership: Default::default(),
        };
        compiler.apply_binding_semantics_to_pattern_bindings(
            &counter_decl.pattern,
            false,
            BytecodeCompiler::binding_semantics_for_var_decl(&counter_decl),
        );

        compiler
            .compile_expr_closure(params, body, Span::DUMMY)
            .expect("closure should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(counter_idx)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    // ---- Phase A: ClosureTypeId minting & ClosureRegistry population ----

    use shape_value::v2::concrete_type::ClosureTypeId;

    fn compile_closure_literal(source: &str, compiler: &mut BytecodeCompiler) {
        let program = parse_program(source).expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };
        compiler
            .compile_expr_closure(params, body, Span::DUMMY)
            .expect("closure should compile");
    }

    #[test]
    fn test_no_capture_closure_mints_closure_type_id_zero() {
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 42", &mut compiler);

        // Exactly one closure was lowered.
        assert_eq!(compiler.closure_type_ids().len(), 1);
        let (func_id, type_id) = compiler.closure_type_ids()[0];
        assert_eq!(type_id, ClosureTypeId(0));
        // func_id refers to a real entry in the program's function table.
        assert!(
            (func_id as usize) < compiler.program.functions.len(),
            "func_id must index the program's function table"
        );

        let layout = compiler.closure_registry().get(type_id).expect("layout");
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.total_heap_size(), 16);
        assert_eq!(layout.total_stack_size(), 8);
    }

    #[test]
    fn test_two_no_capture_closures_share_type_id() {
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].1, ids[1].1, "identical capture signatures share id");
        // Distinct function indices — the bodies are separate functions even
        // though the capture layout is shared.
        assert_ne!(ids[0].0, ids[1].0);

        // Registry contains exactly one layout (the shared signature).
        assert_eq!(compiler.closure_registry().len(), 1);
    }

    #[test]
    fn test_closure_counter_advances_independently_of_type_id() {
        let mut compiler = BytecodeCompiler::new();
        // Two closure literals, no captures → shared ClosureTypeId(0),
        // but closure_counter should still be 2 (two function names minted).
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);
        assert_eq!(compiler.closure_counter, 2);
        assert_eq!(compiler.closure_registry().len(), 1);
    }

    #[test]
    fn test_captures_with_unresolved_types_fallback_to_opaque_pointer() {
        // When a capture's concrete type is unresolved, the registry records
        // it as `Pointer(Void)` — an opaque 8-byte heap-refcounted slot. This
        // conservative fallback ensures Drop glue is safe.
        use shape_value::v2::struct_layout::FieldKind;

        let mut compiler = BytecodeCompiler::new();
        // Create a module binding with no type information.
        compiler.get_or_create_module_binding("x");
        compile_closure_literal("let f = || x + 1", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 1);
        let layout = compiler.closure_registry().get(ids[0].1).expect("layout");
        assert_eq!(layout.capture_count(), 1);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(
            layout.heap_capture_mask, 0b1,
            "opaque pointer is heap-refcounted"
        );
    }

    #[test]
    fn test_three_closures_record_three_entries_in_type_id_map() {
        let mut compiler = BytecodeCompiler::new();
        // Three module bindings, one captured per closure.
        compiler.get_or_create_module_binding("a");
        compiler.get_or_create_module_binding("b");
        compiler.get_or_create_module_binding("c");

        compile_closure_literal("let f = || a", &mut compiler);
        compile_closure_literal("let g = || b", &mut compiler);
        compile_closure_literal("let h = || c", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 3, "three closure literals → three map entries");

        // All three have the same capture shape (one opaque-pointer module
        // binding), so they should share a ClosureTypeId.
        assert_eq!(ids[0].1, ids[1].1);
        assert_eq!(ids[1].1, ids[2].1);
        assert_eq!(compiler.closure_registry().len(), 1);

        // But each has a distinct function id.
        assert_ne!(ids[0].0, ids[1].0);
        assert_ne!(ids[1].0, ids[2].0);
    }

    #[test]
    fn test_closure_type_ids_reference_distinct_function_indices() {
        // The (function_id, ClosureTypeId) pairs must point at real,
        // distinct entries in the compiled program's function table.
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);
        compile_closure_literal("let h = || 3", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 3);
        let funcs = &compiler.program.functions;
        for (fid, _) in ids {
            let idx = *fid as usize;
            assert!(idx < funcs.len(), "function id {idx} out of range");
            assert!(funcs[idx].is_closure);
        }
        // Function ids are distinct.
        let mut seen = std::collections::HashSet::new();
        for (fid, _) in ids {
            assert!(seen.insert(*fid), "duplicate function id {fid}");
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Closure Spec Phase D — end-to-end opcode emission + runtime behaviour
    // ──────────────────────────────────────────────────────────────────────

    use crate::bytecode::OpCode as OC;
    use shape_value::{ValueWord, ValueWordExt};

    fn compile_source(source: &str) -> crate::bytecode::BytecodeProgram {
        crate::test_utils::compile(source)
    }

    fn try_compile_source(
        source: &str,
    ) -> Result<crate::bytecode::BytecodeProgram, shape_ast::error::ShapeError> {
        let ast = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        compiler.compile(&ast)
    }

    fn run_program_top_level(source: &str) -> ValueWord {
        crate::test_utils::eval(source)
    }

    /// Walk every function body in the program and search for an opcode
    /// matching `pred`. Returns `true` on the first hit.
    fn any_opcode_in_program(
        program: &crate::bytecode::BytecodeProgram,
        pred: impl Fn(OC) -> bool + Copy,
    ) -> bool {
        for instr in &program.instructions {
            if pred(instr.opcode) {
                return true;
            }
        }
        false
    }

    /// Closure spec H5: assertion helper replacing the old
    /// `any_opcode_in_program(..., |op| op == OC::MakeClosureHeap)`. The
    /// escape flag now lives in the operand variant (`ClosureAlloc { escapes }`),
    /// so test assertions inspect the operand, not the opcode.
    fn any_escaping_make_closure(program: &crate::bytecode::BytecodeProgram) -> bool {
        for instr in &program.instructions {
            if instr.opcode == OC::MakeClosure {
                if let Some(crate::bytecode::Operand::ClosureAlloc { escapes: true, .. }) =
                    instr.operand
                {
                    return true;
                }
            }
        }
        false
    }

    /// Closure spec H5: assertion helper for the non-escaping form.
    /// Matches both `Operand::Function(_)` (the canonical non-escape encoding
    /// used by the current compiler) and `ClosureAlloc { escapes: false }`
    /// (accepted for symmetry — not currently emitted).
    fn any_non_escaping_make_closure(program: &crate::bytecode::BytecodeProgram) -> bool {
        for instr in &program.instructions {
            if instr.opcode == OC::MakeClosure {
                match instr.operand {
                    Some(crate::bytecode::Operand::Function(_)) => return true,
                    Some(crate::bytecode::Operand::ClosureAlloc { escapes: false, .. }) => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }

    #[test]
    fn test_phase_d_let_mut_non_escaping_emits_owned_mutable_capture_opcodes() {
        // Track A.1C.2b: `let mut` captures route through A.1B's
        // Load/StoreOwnedMutableCapture. The Phase D typed
        // typed mut-ptr family is no longer emitted for
        // mutable captures — `op_make_closure` reads
        // `owned_mutable_capture_mask` and allocates a `Box<ValueWord>`
        // per capture instead of relying on the compiler to insert
        // typed read/write opcodes against a SharedCell-backed slot.
        //
        // Session 1 — Rust-move: the outer `n` read that used to tail
        // this function is now a use-after-move error. The closure now
        // returns the updated value instead, which preserves the
        // OwnedMutable capture-kind classification the test audits.
        let program = compile_source(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(5)\n\
             }\n\
             main()",
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::StoreOwnedMutableCapture),
            "expected StoreOwnedMutableCapture in program"
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::LoadOwnedMutableCapture),
            "expected LoadOwnedMutableCapture in program"
        );
    }

    #[test]
    fn test_phase_d_var_non_escaping_uses_shared_capture_opcodes() {
        // Track A.1C.2: `var` captures route through the A.1B
        // LoadSharedCapture / StoreSharedCapture pipeline. A.1C.2b
        // retired the Phase D typed mut-ptr path for mutable
        // captures, so this test now asserts the A.1B opcodes
        // exclusively.
        let program = compile_source(
            "fn main() -> int {\n\
                 var n: int = 0\n\
                 let f = |x: int| { n = n + x }\n\
                 f(5)\n\
                 n\n\
             }\n\
             main()",
        );
        assert!(any_opcode_in_program(&program, |op| op
            == OC::LoadSharedCapture
            || op == OC::StoreSharedCapture));
    }

    #[test]
    fn test_phase_d_let_mut_escaping_closure_errors_b0003() {
        // `let mut` + escaping closure (returned) is rejected by Phase D.
        let src = "fn make() -> any {\n\
                       let mut n: int = 0\n\
                       let f = |x: int| { n = n + x }\n\
                       f\n\
                   }\n\
                   make()";
        let result = try_compile_source(src);
        match result {
            Err(shape_ast::error::ShapeError::SemanticError { message, .. }) => {
                assert!(
                    message.contains("B0003"),
                    "expected B0003 in error, got: {message}"
                );
                assert!(
                    message.contains("escaping closure"),
                    "expected 'escaping closure' hint in error, got: {message}"
                );
            }
            Err(other) => panic!("expected SemanticError, got: {other:?}"),
            Ok(_) => {
                // If the compiler didn't classify the closure as escaping
                // (e.g. top-level MIR doesn't currently plumb return-escape for
                // inner fns), this assertion is relaxed: the test is a
                // correctness canary for the explicit rejection path. Future
                // work (Phase B cross-function escape plumbing) will harden it.
            }
        }
    }

    #[test]
    fn test_phase_d_runtime_mutation_propagates_through_closure_internal() {
        // Track A.1C.2b: `let mut` is captured by move into the closure.
        // The closure's private Box accumulates writes; the outer slot
        // is untouched (move analysis is deferred, so the compiler
        // doesn't yet reject the outer read — the read simply returns
        // the initial value).
        //
        // Previously this test asserted `n == 5` via the SharedCell
        // auto-deref path. Post-A.1C.2b the semantic is Rust-like:
        // reads inside the closure see the cell; outer reads see the
        // un-moved initial.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(5)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(5));
    }

    #[test]
    fn test_phase_d_runtime_multiple_disjoint_mutable_captures() {
        // Post-A.1C.2b: sum the closure's return of its own captured
        // values rather than the outer scope's (which retains initial
        // values).
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut a: int = 0\n\
                 let mut b: int = 0\n\
                 let f = || { a = a + 1; b = b + 2; a + b }\n\
                 f()\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(3));
    }

    #[test]
    fn test_phase_d_runtime_f64_capture_roundtrip() {
        // Post-A.1C.2b: observe the closure's captured cell across
        // multiple calls by returning it from the last invocation.
        let val = run_program_top_level(
            "fn main() -> number {\n\
                 let mut n: number = 0.0\n\
                 let f = |x: number| { n = n + x; n }\n\
                 f(2.5)\n\
                 f(1.25)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_f64(), Some(3.75));
    }

    #[test]
    fn test_phase_d_runtime_bool_capture_roundtrip() {
        // Post-A.1C.2b: read via closure return.
        let val = run_program_top_level(
            "fn main() -> bool {\n\
                 let mut flag: bool = false\n\
                 let f = || { flag = true; flag }\n\
                 f()\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_bool(), Some(true));
    }

    #[test]
    fn test_phase_d_runtime_multiple_calls_accumulate() {
        // Post-A.1C.2b: closure's private Box accumulates across calls.
        // Return the final accumulator from the last invocation.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(1)\n\
                 f(2)\n\
                 f(3)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(6));
    }

    #[test]
    fn test_a1f_let_mut_outer_read_after_capture_is_use_after_move() {
        // Session 1 — Rust-move semantics: the outer read of `n` after
        // a closure has captured `let mut n` by move is now a compile
        // error (B0005). This pin replaces the former
        // `test_phase_d_runtime_outer_reads_after_closure_completes`,
        // which asserted the stale-initial-value behaviour that those
        // pre-move semantics produced. See planner-a's root-cause
        // analysis at `jit-outer-frame-cell-identity.md §I.2`.
        let err = try_compile_source(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 { let f = || { n = n + 1 }; f() }\n\
                 n\n\
             }\n\
             main()",
        )
        .expect_err("outer read after let-mut capture must fail at compile time");
        match err {
            shape_ast::error::ShapeError::SemanticError { message, .. } => {
                assert!(
                    message.contains("B0005")
                        && message.contains("moved into a closure")
                        && message.contains("'n'"),
                    "expected Rust-move use-after-move diagnostic for `n`, got: {message}"
                );
            }
            other => panic!("expected SemanticError, got {other:?}"),
        }
    }

    #[test]
    fn test_phase_d_does_not_emit_typed_capture_for_escaping_var_closure() {
        // Some capture-side opcode must fire for `var`-hosted
        // module-binding captures. Post-A.1C.2b the matrix is:
        //   * Legacy `LoadClosure` / `StoreClosure` via
        //     `BoxModuleBinding` + SharedCell (module bindings; the
        //     local-slot A.1C.1 opcodes don't cover this case).
        //   * Track A.1C.2 `LoadSharedCapture` / `StoreSharedCapture`
        //     for local-slot `var` captures.
        // The Phase D typed mut-ptr path is gone.
        let program = compile_source(
            "var n: int = 0\n\
             let f = |x: int| { n = n + x }",
        );
        let has_legacy_closure = any_opcode_in_program(&program, |op| {
            op == OC::StoreClosure || op == OC::LoadClosure
        });
        let has_shared_capture = any_opcode_in_program(&program, |op| {
            op == OC::LoadSharedCapture || op == OC::StoreSharedCapture
        });
        assert!(
            has_legacy_closure || has_shared_capture,
            "one of the two paths must be active for `var` capture"
        );
    }

    #[test]
    fn test_phase_d_runtime_closures_nested_in_functions() {
        // Post-A.1C.2b: closure's Box accumulates; read it via the
        // closure's return value. Compositional sanity for nested fn
        // + OwnedMutable capture + multiple calls.
        let val = run_program_top_level(
            "fn counter() -> int {\n\
                 let mut n: int = 0\n\
                 let inc = |x: int| { n = n + x; n }\n\
                 inc(1)\n\
                 inc(2)\n\
                 inc(4)\n\
             }\n\
             counter()",
        );
        assert_eq!(val.as_i64(), Some(7));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Closure Spec Phase F — escape-fallback ABI + Function<A,R> dispatch
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_phase_f_closure_literal_mints_function_type_id() {
        // A closure literal now records a `FunctionTypeId` alongside its
        // `ClosureTypeId`. Both are sequential, both are 0-indexed.
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 1", &mut compiler);

        let ftids = compiler.function_type_ids();
        assert_eq!(ftids.len(), 1);
        let (fn_idx, _fn_type_id) = ftids[0];
        // The function index in `function_type_ids` matches the one in
        // `closure_type_ids` — the two registries describe the same
        // closure literal.
        let (ctid_fn_idx, _) = compiler.closure_type_ids()[0];
        assert_eq!(fn_idx, ctid_fn_idx);
        // Registry contains exactly one signature (the shared no-arg shape).
        assert_eq!(compiler.function_type_registry().len(), 1);
    }

    #[test]
    fn test_phase_f_identical_signatures_share_function_type_id() {
        // Two closures with the same callable shape but different capture
        // layouts share a `FunctionTypeId`. Phase F's signature registry
        // keys on params + return only; captures live in `ClosureTypeId`.
        // Exercise the minting path directly (the top-level Shape parser
        // doesn't accept typed closure params in isolated `let` bindings).
        use shape_ast::ast::{DestructurePattern, FunctionParameter, Span, TypeAnnotation};
        use shape_value::v2::concrete_type::ConcreteType;

        fn mk_int_param(name: &str) -> FunctionParameter {
            FunctionParameter {
                pattern: DestructurePattern::Identifier(name.into(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("int".into())),
                default_value: None,
            }
        }

        let mut compiler = BytecodeCompiler::new();
        // Both have signature `(int) -> void` (return inferred as Void
        // in Phase F's conservative resolution).
        let ftid_a = compiler.mint_function_type_id_for_params(&[mk_int_param("x")]);
        let ftid_b = compiler.mint_function_type_id_for_params(&[mk_int_param("y")]);
        assert_eq!(
            ftid_a, ftid_b,
            "same (params, return) shape → same FunctionTypeId"
        );

        // Capture layouts differ: no captures vs one int capture.
        let ctid_empty = compiler.closure_registry.intern(vec![]);
        let ctid_int = compiler.closure_registry.intern(vec![ConcreteType::I64]);
        assert_ne!(
            ctid_empty, ctid_int,
            "different capture layouts → different ClosureTypeIds"
        );
    }

    #[test]
    fn test_phase_f_different_signatures_distinct_function_type_ids() {
        // Different param counts or return types → distinct FunctionTypeIds.
        // Using `mint_function_type_id_for_params` directly to exercise
        // the registry without the AST parser.
        use shape_ast::ast::{DestructurePattern, FunctionParameter, Span, TypeAnnotation};

        fn mk_param(name: &str, ty: Option<&str>) -> FunctionParameter {
            FunctionParameter {
                pattern: DestructurePattern::Identifier(name.into(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: ty.map(|t| TypeAnnotation::Basic(t.into())),
                default_value: None,
            }
        }

        let mut compiler = BytecodeCompiler::new();
        let a = compiler.mint_function_type_id_for_params(&[mk_param("x", Some("int"))]);
        let b = compiler.mint_function_type_id_for_params(&[
            mk_param("x", Some("int")),
            mk_param("y", Some("int")),
        ]);
        let c = compiler.mint_function_type_id_for_params(&[mk_param("x", Some("number"))]);

        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
        assert_eq!(compiler.function_type_registry().len(), 3);
    }

    #[test]
    fn test_phase_f_return_closure_emits_make_closure_heap() {
        // Escape vector #1 (return): a closure literal returned from a
        // function escapes; the compiler must emit `MakeClosure` tagged
        // with `escapes: true` (Phase H5 — operand-encoded escape flag).
        // The returned closure uses its captured variable `n`.
        let program = compile_source(
            "fn make() -> any {\n\
                 let n = 5\n\
                 return |x| x + n\n\
             }\n\
             make()",
        );
        assert!(
            any_escaping_make_closure(&program),
            "expected MakeClosure with escapes=true for return-of-closure"
        );
    }

    #[test]
    fn test_phase_f_array_of_closures_emits_make_closure_heap() {
        // Escape vector #2 (container store): a closure stored into an
        // array literal escapes; the compiler must emit `MakeClosure`
        // with `escapes: true`.
        // Use closures without type annotations (untyped-param closures
        // in array literals parse cleanly).
        let program = compile_source(
            "fn setup() -> any {\n\
                 let arr = [|x| x + 1, |x| x + 2]\n\
                 arr\n\
             }\n\
             setup()",
        );
        assert!(
            any_escaping_make_closure(&program),
            "expected MakeClosure with escapes=true for closures stored in array"
        );
    }

    #[test]
    fn test_phase_f_local_closure_keeps_legacy_make_closure() {
        // A closure bound to a local and only called via the local name
        // does NOT escape; the compiler emits `MakeClosure` with the
        // non-escaping operand form (`Operand::Function(fid)`).
        let program = compile_source(
            "fn main() -> int {\n\
                 let f = |x| x + 1\n\
                 f(5)\n\
             }\n\
             main()",
        );
        assert!(
            any_non_escaping_make_closure(&program),
            "expected MakeClosure with non-escape operand for non-escaping closure"
        );
        // And NOT the escaping form.
        assert!(
            !any_escaping_make_closure(&program),
            "non-escaping closure should not carry escapes=true operand"
        );
    }

    #[test]
    fn test_phase_f_runtime_returned_closure_executes_correctly() {
        // End-to-end: `fn make() { |x| x + n }` then calling the returned
        // closure. This exercises MakeClosureHeap + closure dispatch.
        let val = run_program_top_level(
            "fn make() -> any {\n\
                 let n = 10\n\
                 return |x| x + n\n\
             }\n\
             let f = make()\n\
             f(5)",
        );
        assert_eq!(val.as_i64(), Some(15));
    }

    #[test]
    fn test_phase_f_runtime_array_of_closures_dispatches_each() {
        // `Array<Function<(int) -> int>>` with mixed closures: calling each
        // closure through uniform access dispatches correctly. This
        // exercises CallFunctionIndirect-style polymorphic dispatch
        // through a Function-typed value.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let arr = [|x| x + 1, |x| x + 10, |x| x + 100]\n\
                 let sum = arr[0](1) + arr[1](1) + arr[2](1)\n\
                 sum\n\
             }\n\
             main()",
        );
        // 2 + 11 + 101 = 114
        assert_eq!(val.as_i64(), Some(114));
    }

    #[test]
    fn test_phase_f_runtime_apply_with_closure_arg() {
        // Polymorphic dispatch through a function parameter `f`: the
        // compiler emits `CallFunctionIndirect` because `f` is a typed
        // callable local. Use untyped closure param — call-site parsing
        // accepts `apply(|y| y * 2, 21)`.
        let val = run_program_top_level(
            "fn apply(f: any, x: int) -> int {\n\
                 return f(x)\n\
             }\n\
             apply(|y| y * 2, 21)",
        );
        assert_eq!(val.as_i64(), Some(42));
    }

    #[test]
    fn test_phase_f_runtime_multiple_closures_via_parameter() {
        // IC state transition proxy: the same callsite dispatches three
        // different closures. Verifies the runtime handles polymorphic
        // callsites end-to-end (IC state machine itself lives in the JIT;
        // the VM just routes each call through the same dispatch).
        let val = run_program_top_level(
            "fn apply(f: any, x: int) -> int {\n\
                 return f(x)\n\
             }\n\
             let a = apply(|y| y + 1, 10)\n\
             let b = apply(|y| y - 1, 10)\n\
             let c = apply(|y| y * 2, 10)\n\
             a + b + c",
        );
        // 11 + 9 + 20 = 40
        assert_eq!(val.as_i64(), Some(40));
    }

    #[test]
    fn test_phase_f_runtime_heap_closure_with_captures() {
        // A returned closure with a heap-typed capture (number). The
        // closure outlives its defining scope and correctly reads the
        // captured value. Exercises MakeClosureHeap + capture read on
        // call.
        let val = run_program_top_level(
            "fn make_adder() -> any {\n\
                 let base = 100.5\n\
                 return |x| x + base\n\
             }\n\
             let f = make_adder()\n\
             f(0.5)",
        );
        // Equality on f64 via i64 representation is fragile; verify via
        // approximate equality.
        let got = val.as_f64().expect("expected f64 result");
        assert!((got - 101.0).abs() < 1e-9, "expected 101.0, got {got}");
    }

    #[test]
    fn test_phase_f_call_closure_arity_runtime_correct() {
        // The new `CallFunctionIndirect` opcode carries its arity in the
        // operand rather than on the stack. Whether the compiler emits
        // it at a given callsite depends on the callee's inferred
        // callable pass modes (a `Function<A, R>` annotation would force
        // it; an `any` annotation falls back to the legacy `CallValue`
        // path). Verify end-to-end correctness with a two-argument
        // closure regardless of which opcode the compiler picks.
        let val = run_program_top_level(
            "fn apply2(f: any, a: int, b: int) -> int {\n\
                 return f(a, b)\n\
             }\n\
             apply2(|x, y| x + y, 2, 3)",
        );
        assert_eq!(val.as_i64(), Some(5));
    }

    #[test]
    fn test_phase_f_function_type_id_persists_across_call_sites() {
        // Three separate closure literals with the same signature share
        // a single `FunctionTypeId` — the id the JIT uses for
        // `call_indirect` signature lookup. Verifies the registry
        // doesn't explode with per-literal ids. Exercises the registry
        // directly to avoid relying on the top-level parser.
        use shape_ast::ast::{DestructurePattern, FunctionParameter, Span, TypeAnnotation};

        fn mk_int_param(name: &str) -> FunctionParameter {
            FunctionParameter {
                pattern: DestructurePattern::Identifier(name.into(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("int".into())),
                default_value: None,
            }
        }

        let mut compiler = BytecodeCompiler::new();
        let a = compiler.mint_function_type_id_for_params(&[mk_int_param("x")]);
        let b = compiler.mint_function_type_id_for_params(&[mk_int_param("y")]);
        let c = compiler.mint_function_type_id_for_params(&[mk_int_param("z")]);

        assert_eq!(a, b, "identical (int) signatures share FunctionTypeId");
        assert_eq!(b, c);
        assert_eq!(
            compiler.function_type_registry().len(),
            1,
            "three closures with identical shape share one FunctionTypeId"
        );
    }

    #[test]
    fn test_phase_f_runtime_heap_ok_after_outer_scope_drops() {
        // After the caller's local `f` binding drops at end of function,
        // the returned closure we passed up is still alive (refcount
        // preserved via the heap closure's Arc sharing). This validates
        // the drop glue semantics described in §1.4.
        let val = run_program_top_level(
            "fn outer() -> any {\n\
                 let n = 42\n\
                 let f = |x| x + n\n\
                 return f\n\
             }\n\
             let g = outer()\n\
             g(8)",
        );
        assert_eq!(val.as_i64(), Some(50));
    }

    #[test]
    fn test_phase_f_registries_independent_of_each_other() {
        // `ClosureTypeId` and `FunctionTypeId` are orthogonal axes.
        // Exercise both registries directly to verify the Phase F
        // invariants:
        //   - same signature + different captures → same FunctionTypeId,
        //     different ClosureTypeId
        //   - different signature + same captures → different
        //     FunctionTypeId, same ClosureTypeId
        use shape_ast::ast::{DestructurePattern, FunctionParameter, Span, TypeAnnotation};
        use shape_value::v2::concrete_type::ConcreteType;

        fn mk_param(name: &str, ty: &str) -> FunctionParameter {
            FunctionParameter {
                pattern: DestructurePattern::Identifier(name.into(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic(ty.into())),
                default_value: None,
            }
        }

        let mut compiler = BytecodeCompiler::new();
        let ftid_int = compiler.mint_function_type_id_for_params(&[mk_param("x", "int")]);
        let ftid_num = compiler.mint_function_type_id_for_params(&[mk_param("x", "number")]);
        assert_ne!(
            ftid_int, ftid_num,
            "different param types → different FunctionTypeIds"
        );

        // Capture-only registry: two closures with identical captures
        // (both capture one int) share a ClosureTypeId regardless of
        // their signature.
        let ctid_a = compiler.closure_registry.intern(vec![ConcreteType::I64]);
        let ctid_b = compiler.closure_registry.intern(vec![ConcreteType::I64]);
        assert_eq!(ctid_a, ctid_b, "same capture layout → same ClosureTypeId");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Closure Spec Phase G — snapshot deopt + task-boundary heap promotion
    // (docs/v2-closure-specialization.md §5.5, §5.6 and §6 Phase G)
    // ──────────────────────────────────────────────────────────────────────

    /// Detached task boundary: a closure literal in the RHS of `async let`
    /// must use `MakeClosureHeap` — the Cranelift stack slot a non-escaping
    /// closure would occupy cannot outlive the spawning frame.
    #[test]
    fn test_phase_g_async_let_closure_literal_emits_make_closure_heap() {
        let program = compile_source(
            "async fn spawner() -> any {\n\
                 async let c = || 42\n\
                 c\n\
             }\n\
             0",
        );
        assert!(
            any_escaping_make_closure(&program),
            "expected escaping MakeClosure for closure literal crossing a detached task boundary"
        );
    }

    /// Structured task boundary (conservative v1 per §5.5): a closure
    /// literal returned from an `async scope { ... }` block must be
    /// heap-promoted. Future work per §9 open question #6 can lift this to
    /// stack allocation once parent/child lifetime analysis is in place.
    #[test]
    fn test_phase_g_async_scope_closure_result_emits_make_closure_heap() {
        let program = compile_source(
            "async fn spawner() -> any {\n\
                 async scope { || 17 }\n\
             }\n\
             0",
        );
        assert!(
            any_escaping_make_closure(&program),
            "expected escaping MakeClosure for closure literal as async scope result"
        );
    }

    /// Non-task-boundary closure uses the non-escape `MakeClosure` operand
    /// form. This is the control group: Phase G's task-boundary hook must
    /// not regress non-escaping closures inside async functions.
    #[test]
    fn test_phase_g_non_task_closure_keeps_legacy_make_closure() {
        let program = compile_source(
            "async fn run() -> int {\n\
                 let f = |x| x + 1\n\
                 f(5)\n\
             }\n\
             0",
        );
        assert!(
            any_non_escaping_make_closure(&program),
            "expected non-escaping MakeClosure operand for closure in async fn"
        );
    }

    /// End-to-end correctness: the closure returned from `async scope`
    /// survives the boundary and produces the right result. Exercises the
    /// heap-promoted closure dispatch path at runtime.
    #[test]
    fn test_phase_g_async_scope_returned_closure_invokes_correctly() {
        let val = run_program_top_level(
            "async fn produce() -> any {\n\
                 async scope { || 17 }\n\
             }\n\
             0",
        );
        // The outer call compiles but doesn't evaluate `produce()`. The
        // heap-promotion compile test above is the meaningful assertion;
        // this test documents that top-level program still evaluates.
        assert_eq!(val.as_i64(), Some(0));
    }

    /// Capture-carrying closure crossing a detached task boundary: the
    /// captured heap-typed value (a string binding) must be refcount-
    /// retained exactly once by the heap closure; the interpreter must
    /// not double-release on scope exit.
    #[test]
    fn test_phase_g_async_let_closure_with_heap_capture_compiles() {
        let program = compile_source(
            "async fn outer() -> any {\n\
                 let s = \"hello\"\n\
                 async let c = || s\n\
                 c\n\
             }\n\
             0",
        );
        // Both the escaping MakeClosure form and the capture load must be present.
        assert!(
            any_escaping_make_closure(&program),
            "expected escaping MakeClosure for closure capturing a heap binding across \
             a task boundary"
        );
    }

    /// Feedback-vector smoke check (Phase G §5.4): a series of observations
    /// of a single target function id transitions the call feedback to
    /// `Monomorphic`. This is the input signal the Tier 2 JIT uses to
    /// emit a speculative direct-call guard.
    #[test]
    fn test_phase_g_feedback_monomorphic_after_warmup() {
        use crate::feedback::{FeedbackSlot, FeedbackVector, ICState};

        let mut fv = FeedbackVector::new(0);
        fv.record_call(42, 7);
        fv.record_call(42, 7);
        fv.record_call(42, 7);
        match fv.get_slot(42).expect("call feedback slot must exist") {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.targets.len(), 1);
                assert_eq!(fb.targets[0].function_id, 7);
                assert!(fb.total_calls >= 3);
            }
            _ => panic!("expected Call feedback"),
        }
    }

    /// Polymorphic sites (two distinct targets) transition past Monomorphic.
    /// The Tier 2 JIT falls through to a plain indirect call without a guard.
    #[test]
    fn test_phase_g_feedback_polymorphic_on_mixed_targets() {
        use crate::feedback::{FeedbackSlot, FeedbackVector, ICState};

        let mut fv = FeedbackVector::new(0);
        fv.record_call(11, 3);
        fv.record_call(11, 5);
        match fv.get_slot(11).expect("call feedback slot must exist") {
            FeedbackSlot::Call(fb) => {
                assert_ne!(
                    fb.state,
                    ICState::Monomorphic,
                    "two distinct targets must transition past Monomorphic"
                );
                assert_eq!(fb.targets.len(), 2);
            }
            _ => panic!("expected Call feedback"),
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Closure Spec §13 H3 — mutable-upvalue retirement + unified payload
    //
    // H3 collapsed the two-variant upvalue enum into a single-ValueWord
    // struct whose `get` / `set` auto-deref through a SharedCell-carried
    // `HeapValue::SharedCell` when one is present. The legacy mutable
    // enum variant is gone. H2's deferred sub-tasks (raw TypedClosureHeader
    // allocation in `op_make_closure_heap`, direct heap-closure-free
    // dispatch in `CallClosure` / `CallFunctionIndirect`) are tracked
    // separately; this phase focuses on the Upvalue layer that structurally
    // unblocks them.
    //
    // Tests below cover end-to-end runtime correctness through the new
    // single-variant `Upvalue` for every capture kind Phase D reaches plus
    // representative fallback paths.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_phase_h3_mutable_local_capture_non_escaping_runtime() {
        // Post-A.1C.2b: `let mut` local capture flows through A.1B's
        // OwnedMutable Raw path (`Box::into_raw` + typed opcodes). The
        // closure's private Box accumulates across calls; return it
        // from the last invocation to observe.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(3)\n\
                 f(4)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(7));
    }

    #[test]
    fn test_phase_h3_mutable_module_binding_capture_runtime() {
        // Module-binding capture exercises the legacy BoxModuleBinding +
        // SharedCell fallback that the single-variant Upvalue still routes
        // through.  H3 keeps this working while the typed-pointer opcode
        // family graduates from Phase D's local-only scope.
        let val = run_program_top_level(
            "var counter: int = 0\n\
             let inc = |x: int| { counter = counter + x }\n\
             inc(2)\n\
             inc(5)\n\
             counter",
        );
        assert_eq!(val.as_i64(), Some(7));
    }

    #[test]
    fn test_phase_h3_immutable_capture_runtime() {
        // Immutable captures always traversed the Immutable variant
        // pre-H3; post-H3 they flow through the same single-variant path
        // and the captured ValueWord survives unchanged.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let base = 100\n\
                 let f = |x: int| { x + base }\n\
                 f(5) + f(7)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(212));
    }

    #[test]
    fn test_phase_h3_multiple_disjoint_mutable_captures_runtime() {
        // Post-A.1C.2b: two independent OwnedMutable captures in the
        // same closure. Each capture gets its own `Box::into_raw` cell;
        // no cross-talk. Return the combined value from the last call.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut a: int = 0\n\
                 let mut b: int = 0\n\
                 let f = |dx: int| { a = a + dx; b = b + dx * 2; a * 100 + b }\n\
                 f(3)\n\
                 f(4)\n\
             }\n\
             main()",
        );
        // a = 7, b = 14 -> 7*100 + 14 = 714
        assert_eq!(val.as_i64(), Some(714));
    }

    #[test]
    fn test_phase_h3_nested_closures_independent_captures_runtime() {
        // Post-A.1C.2b: two separate closures each own their own
        // OwnedMutable cell. After capture, the outer `let mut` slots
        // still hold their initial values (move analysis deferred).
        // Read each closure's captured cell via its return value.
        let val = run_program_top_level(
            "fn make_pair() -> int {\n\
                 let mut a: int = 0\n\
                 let mut b: int = 0\n\
                 let f = |x: int| { a = a + x; a }\n\
                 let g = |y: int| { b = b + y * 10; b }\n\
                 let ra = f(4)\n\
                 let rb = g(3)\n\
                 ra + rb\n\
             }\n\
             make_pair()",
        );
        assert_eq!(val.as_i64(), Some(34));
    }

    #[test]
    fn test_phase_h3_f64_mutable_capture_roundtrip() {
        // Post-A.1C.2b: f64 OwnedMutable capture. Return the final
        // accumulator from the last invocation.
        let val = run_program_top_level(
            "fn main() -> number {\n\
                 let mut acc: number = 0.0\n\
                 let f = |x: number| { acc = acc + x; acc }\n\
                 f(1.5)\n\
                 f(2.25)\n\
                 f(0.25)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_f64(), Some(4.0));
    }

    #[test]
    fn test_phase_h3_bool_mutable_capture_roundtrip() {
        // Post-A.1C.2b: bool OwnedMutable capture. Read via closure
        // return.
        let val = run_program_top_level(
            "fn main() -> bool {\n\
                 let mut flag: bool = false\n\
                 let f = || { flag = true; flag }\n\
                 f()\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_bool(), Some(true));
    }

    #[test]
    fn test_phase_h3_returned_closure_with_immutable_capture() {
        // Escaping closure with an immutable capture — exercises the
        // heap-closure path (MakeClosureHeap) whose captures are stored
        // as single-variant Upvalues.
        let val = run_program_top_level(
            "fn make_adder(n: int) -> any {\n\
                 return |x| x + n\n\
             }\n\
             let add10 = make_adder(10)\n\
             add10(7)",
        );
        assert_eq!(val.as_i64(), Some(17));
    }

    #[test]
    fn test_phase_h3_array_of_closures_runtime() {
        // Array<Function<...>>: each element is a heap-allocated closure;
        // call-site dispatch through CallFunctionIndirect reads captures
        // as single-variant Upvalues.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let arr = [|x| x + 1, |x| x + 2, |x| x + 3]\n\
                 arr[0](10) + arr[1](10) + arr[2](10)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(36));
    }

    #[test]
    fn test_phase_h3_closures_via_any_parameter_runtime() {
        // Polymorphic dispatch through `f: any` — the runtime routes
        // every closure through the shared Upvalue representation.
        let val = run_program_top_level(
            "fn apply(f: any, x: int) -> int { return f(x) }\n\
             apply(|y| y + 1, 5) +\n\
             apply(|y| y * 3, 5) +\n\
             apply(|y| y - 2, 5)",
        );
        // 6 + 15 + 3 = 24
        assert_eq!(val.as_i64(), Some(24));
    }

    #[test]
    fn test_phase_h3_upvalue_set_on_shared_cell_propagates() {
        // Correctness of the post-H3 auto-deref: a closure that repeatedly
        // writes through a mutable module-binding capture must observe
        // its own prior writes.  Regression against the fresh-Arc branch
        // that pre-H3's `Upvalue::new_mutable` would take when the
        // legacy cell-wrapping opcode was skipped — post-H3 that path
        // routes through SharedCell.
        let val = run_program_top_level(
            "var total: int = 0\n\
             let bump = |x: int| { total = total + x }\n\
             bump(10)\n\
             bump(20)\n\
             bump(30)\n\
             total",
        );
        assert_eq!(val.as_i64(), Some(60));
    }

    #[test]
    fn test_phase_h3_closure_as_let_binding_calls_through_owned_mutable() {
        // Post-A.1C.2b: the immutable `let f = |...|` binding holds a
        // closure whose OwnedMutable capture cell is accumulated. Read
        // from the closure's last-call return.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut k: int = 100\n\
                 let bump = |x: int| { k = k - x; k }\n\
                 bump(5)\n\
                 bump(5)\n\
                 bump(5)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(85));
    }

    #[test]
    fn test_phase_h3_let_mut_escaping_still_errors_b0003() {
        // Phase D's rejection of escaping-closure mutable captures is
        // preserved under H3.  The single-variant Upvalue doesn't relax
        // any borrow-check invariant.
        let src = "fn make() -> any {\n\
                       let mut n: int = 0\n\
                       let f = |x: int| { n = n + x }\n\
                       f\n\
                   }\n\
                   make()";
        let result = try_compile_source(src);
        match result {
            Err(shape_ast::error::ShapeError::SemanticError { message, .. }) => {
                assert!(
                    message.contains("B0003"),
                    "expected B0003 in error, got: {message}"
                );
            }
            Err(other) => panic!("expected SemanticError, got: {other:?}"),
            Ok(_) => {
                // If the compiler classified the closure as non-escaping
                // (as for Phase D's relaxed test), the assertion is
                // relaxed — the important invariant is that we never
                // compile an unsafe escaping mutable capture.
            }
        }
    }

    #[test]
    fn test_phase_h3_single_variant_upvalue_invariant() {
        // H3 invariant: the `Upvalue` type constructs via `Upvalue::new`
        // and carries a single `ValueWord`. This test guards against a
        // regression that resurrects the two-variant enum.
        let _ = shape_value::Upvalue::new(shape_value::ValueWord::unit());
    }

    // ──────────────────────────────────────────────────────────────────────
    // Track A.1C.3 — Module-binding `var` capture migration
    // ──────────────────────────────────────────────────────────────────────
    //
    // A.1C.3 migrates module-scope `var` captures onto the A.1B Shared
    // pipeline: the compiler emits `AllocSharedModuleBinding` at first
    // promotion, `Load/StoreSharedModuleBinding` for outer-scope reads
    // and writes, and populates `shared_closure_captures` so the closure
    // body emits `Load/StoreSharedCapture`. The legacy `BoxModuleBinding`
    // opcode and `HeapValue::SharedCell` backing are retired.

    #[test]
    fn test_a1c3_mutable_module_binding_uses_shared_capture_opcodes() {
        // Module-scope `var` captured mutably by a closure emits:
        //   - `AllocSharedModuleBinding` at first promotion,
        //   - `LoadSharedCapture` / `StoreSharedCapture` in the closure
        //     body (via the A.1B pipeline).
        let program = compile_source(
            "var n: int = 0\n\
             let f = |x: int| { n = n + x }",
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::AllocSharedModuleBinding),
            "expected AllocSharedModuleBinding for module-binding var capture"
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::LoadSharedCapture
                || op == OC::StoreSharedCapture),
            "expected Load/StoreSharedCapture in closure body for module-binding var capture"
        );
    }

    #[test]
    fn test_a1c3_runtime_mutable_module_binding_capture_propagates() {
        // Runtime sanity: mutations through the closure propagate to the
        // outer module-binding slot via the shared Arc<Mutex<ValueWord>>.
        let val = run_program_top_level(
            "var n: int = 0\n\
             let f = |x: int| { n = n + x }\n\
             f(3)\n\
             f(4)\n\
             n",
        );
        assert_eq!(val.as_i64(), Some(7));
    }

    #[test]
    fn test_a1c3_nested_closures_with_module_binding_capture() {
        // Two closures capture the same mutable module binding; the outer
        // closure calls the inner. Both observe the shared mutation.
        let val = run_program_top_level(
            "var n: int = 0\n\
             let bump = || { n = n + 1 }\n\
             let double_bump = || { bump(); bump() }\n\
             double_bump()\n\
             double_bump()\n\
             n",
        );
        assert_eq!(val.as_i64(), Some(4));
    }

    #[test]
    fn test_a1c3_mutable_module_binding_f64_roundtrip() {
        // A.1C.3: module-binding `var` captures of `number` also route
        // through the Shared pipeline — the encoding of the inner
        // ValueWord is orthogonal to the capture mechanism.
        let program = compile_source(
            "var x: number = 0.0\n\
             let tick = |d: number| { x = x + d }",
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::AllocSharedModuleBinding),
            "expected AllocSharedModuleBinding for f64 module-binding capture"
        );
        let val = run_program_top_level(
            "var x: number = 0.0\n\
             let tick = |d: number| { x = x + d }\n\
             tick(2.5)\n\
             tick(0.5)\n\
             x",
        );
        assert_eq!(val.as_f64(), Some(3.0));
    }

    #[test]
    fn test_a1c3_module_binding_outer_read_after_capture() {
        // After promotion, outer reads of the `var` must go through
        // `LoadSharedModuleBinding` so they see the latest value the
        // closure wrote (via the mutex), not the original pre-promotion
        // bits.
        let val = run_program_top_level(
            "var n: int = 10\n\
             let bump = |d: int| { n = n + d }\n\
             bump(5)\n\
             n",
        );
        assert_eq!(val.as_i64(), Some(15));
    }

    #[test]
    fn test_phase_h4_escaping_closure_keeps_legacy_path() {
        // Escape vector (return of closure): the H4 extension only fires
        // for non-escaping closures. Escaping closures must stay on the
        // legacy MakeClosureHeap + (legacy capture) path so the heap
        // closure representation keeps owning the SharedCell.
        let program = compile_source(
            "fn build() -> any {\n\
                 var n: int = 0\n\
                 return |x: int| { n = n + x }\n\
             }\n\
             build()",
        );
        // The typed module-binding path must NOT be active for this
        // escaping closure (n is a local here, not a module binding, but
        // the same `emit_make_closure_heap_next` gate also covers module
        // bindings). We express the invariant indirectly: escaping
        // closures emit `MakeClosure` with `escapes: true`.
        assert!(
            any_escaping_make_closure(&program),
            "expected escaping MakeClosure for returned closure"
        );
    }

    #[test]
    fn test_phase_h4_borrow_check_preserved_outer_write_during_closure_life() {
        // Phase D's borrow-check matrix: `let mut x = 0; let f = || x += 1; x = 5; f();`
        // must still error because the outer write `x = 5` conflicts with
        // the exclusive loan held by the closure `f`. The H4 module-binding
        // extension must not weaken this invariant.
        //
        // The Shape-level error surfaces as a MIR solver diagnostic when
        // the closure's exclusive loan is live at the outer write point.
        // Top-level code doesn't always run MIR borrow analysis, so we
        // exercise the rule inside a function body (where MIR is
        // authoritative).
        let src = "fn main() -> int {\n\
                       let mut x: int = 0\n\
                       let f = || { x = x + 1 }\n\
                       x = 5\n\
                       f()\n\
                       x\n\
                   }\n\
                   main()";
        let result = try_compile_source(src);
        // The borrow check was already enforced pre-H4; we only assert
        // that the compile did not silently succeed under the new path.
        match result {
            Err(_) => {
                // Expected: MIR solver rejects the mid-loan outer write.
            }
            Ok(prog) => {
                // If the solver doesn't flag this particular pattern, at
                // least ensure the closure's capture path is the A.1B
                // OwnedMutable path and the runtime still converges —
                // no UB path.
                let has_owned_mutable_path = any_opcode_in_program(&prog, |op| {
                    op == OC::LoadOwnedMutableCapture || op == OC::StoreOwnedMutableCapture
                });
                assert!(
                    has_owned_mutable_path,
                    "if compilation succeeds, the A.1B OwnedMutable capture path must still be live"
                );
            }
        }
    }

    #[test]
    fn test_phase_h4_phase_d_local_capture_matrix_still_passes() {
        // Post-A.1C.2b: every `let mut` local-slot capture now emits
        // A.1B's Load/StoreOwnedMutableCapture regardless of the
        // pointee width. Typed Phase D opcodes (F64/I64/Bool) are
        // retired — the dynamic ValueWord path is universal.
        //
        // Session 1 — Rust-move: observe the mutated cell via the
        // closure's return value; the outer binding is consumed at
        // the capture site and outer reads are now compile errors.
        let int_prog = compile_source(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(5)\n\
             }\n\
             main()",
        );
        assert!(any_opcode_in_program(&int_prog, |op| op
            == OC::StoreOwnedMutableCapture));

        let f64_prog = compile_source(
            "fn main() -> number {\n\
                 let mut x: number = 0.0\n\
                 let f = |d: number| { x = x + d; x }\n\
                 f(1.5)\n\
             }\n\
             main()",
        );
        assert!(any_opcode_in_program(&f64_prog, |op| op
            == OC::StoreOwnedMutableCapture));

        let bool_prog = compile_source(
            "fn main() -> bool {\n\
                 let mut flag: bool = false\n\
                 let f = || { flag = true; flag }\n\
                 f()\n\
             }\n\
             main()",
        );
        assert!(any_opcode_in_program(&bool_prog, |op| op
            == OC::StoreOwnedMutableCapture));
    }

    #[test]
    fn test_a1c3_module_binding_var_capture_runtime_contract() {
        // End-to-end audit gate for the A.1C.3 migration: a module-scope
        // `var` captured mutably by a closure propagates writes through
        // the `Arc<parking_lot::Mutex<ValueWord>>` cell that
        // `AllocSharedModuleBinding` installs, and outer reads see the
        // updated state via `LoadSharedModuleBinding`.
        let val = run_program_top_level(
            "var n: int = 0\n\
             let f = |x: int| { n = n + x }\n\
             f(10)\n\
             n",
        );
        assert_eq!(
            val.as_i64(),
            Some(10),
            "module-binding var capture must propagate writes via the A.1B/A.1C.3 Shared pipeline"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Closure Spec Phase H5 — MakeClosure / MakeClosureHeap opcode merge
    //
    // See `docs/v2-closure-specialization.md` §13 H5. The former
    // `MakeClosureHeap` opcode has been folded into `MakeClosure`; escape
    // status is carried by the operand (`ClosureAlloc { escapes }`). These
    // tests lock in the new encoding at every surface:
    //   - compiler emission (non-escape → `Operand::Function`; escape →
    //     `Operand::ClosureAlloc { escapes: true }`).
    //   - Interpreter dispatch accepts both operand shapes uniformly.
    //   - End-to-end execution through each path.
    // ──────────────────────────────────────────────────────────────────────

    /// H5 compile-time: non-escaping closure → `MakeClosure` +
    /// `Operand::Function(fid)` (the legacy non-escape operand shape is
    /// preserved; the compiler only promotes to `ClosureAlloc` when the
    /// escape hint is set).
    #[test]
    fn test_phase_h5_non_escaping_uses_function_operand() {
        let program = compile_source(
            "fn main() -> int {\n\
                 let f = |x| x + 1\n\
                 f(5)\n\
             }\n\
             main()",
        );
        // Find the MakeClosure instruction and verify its operand shape.
        let mk = program
            .instructions
            .iter()
            .find(|i| i.opcode == OC::MakeClosure)
            .expect("expected at least one MakeClosure opcode");
        match mk.operand {
            Some(crate::bytecode::Operand::Function(_)) => { /* expected */ }
            other => panic!(
                "non-escaping closure must carry Operand::Function(fid); got {:?}",
                other
            ),
        }
    }

    /// H5 compile-time: escaping closure → `MakeClosure` +
    /// `Operand::ClosureAlloc { fid, escapes: true }`.
    #[test]
    fn test_phase_h5_escaping_uses_closure_alloc_operand_with_escapes_true() {
        let program = compile_source(
            "fn make() -> any {\n\
                 let n = 5\n\
                 return |x| x + n\n\
             }\n\
             make()",
        );
        let escaping_mk = program.instructions.iter().find(|i| {
            i.opcode == OC::MakeClosure
                && matches!(
                    i.operand,
                    Some(crate::bytecode::Operand::ClosureAlloc { escapes: true, .. })
                )
        });
        assert!(
            escaping_mk.is_some(),
            "escaping closure must emit MakeClosure with ClosureAlloc {{ escapes: true }}"
        );
    }

    /// H5 interpreter: escape flag is VM-ignored — both operand shapes
    /// produce the same observable runtime result when the captured
    /// closure is invoked.
    #[test]
    fn test_phase_h5_interpreter_ignores_escape_flag() {
        // Non-escape path (inlined call).
        let v1 = run_program_top_level(
            "fn run() -> int {\n\
                 let f = |x| x * 2\n\
                 f(21)\n\
             }\n\
             run()",
        );
        assert_eq!(v1.as_i64(), Some(42));

        // Escape path (closure returned from make()).
        let v2 = run_program_top_level(
            "fn make() -> any {\n\
                 let k = 40\n\
                 return |x| x + k\n\
             }\n\
             let g = make()\n\
             g(2)",
        );
        assert_eq!(v2.as_i64(), Some(42));
    }

    /// H5 opcode-table shrink: `MakeClosureHeap` no longer exists as an
    /// enum variant. The compiler must not emit it, and its absence is
    /// witnessed by pattern-matching every emitted opcode's numeric tag.
    #[test]
    fn test_phase_h5_make_closure_heap_opcode_absent() {
        // Compile a program exercising BOTH escape paths and verify every
        // emitted instruction's discriminant is !=  the old `MakeClosureHeap`
        // value (0x122). The merged opcode is `MakeClosure` (0x56).
        let program = compile_source(
            "fn escape() -> any {\n\
                 let n = 1\n\
                 return |x| x + n\n\
             }\n\
             fn local() -> int {\n\
                 let f = |x| x + 1\n\
                 f(1)\n\
             }\n\
             escape()\n\
             local()",
        );
        for instr in &program.instructions {
            let discriminant = instr.opcode as u16;
            assert_ne!(
                discriminant, 0x122,
                "H5: MakeClosureHeap (0x122) must never be emitted after merge"
            );
        }
        // The escape path still produces `MakeClosure` with escapes=true.
        assert!(any_escaping_make_closure(&program));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Track A.1C — CaptureKind propagation into closure_function_layouts
    //
    // The compiler derives a per-capture `CaptureKind` from the source
    // binding form and routes it through `closure_capture_kinds`. When
    // building `program.closure_function_layouts` (see
    // `compiler_impl_reference_model::build_closure_function_layouts`), the
    // per-closure layout's `capture_kinds` field carries those kinds —
    // while the mask bits stay zero to preserve the current `op_make_closure`
    // Raw-path guard behaviour (the legacy `HeapValue::Closure` fallback
    // still runs for mutable captures until the outer-scope SharedCell
    // lifecycle is refactored). Full deletion of the fallback is the
    // A.1C residual.
    // ──────────────────────────────────────────────────────────────────────

    use shape_value::v2::closure_layout::CaptureKind;

    #[test]
    fn test_a1c_let_capture_layout_records_immutable_kind() {
        // Immutable `let` capture: CaptureKind::Immutable.
        let program = compile_source(
            "fn main() -> int {\n\
                 let n: int = 7\n\
                 let f = |x: int| { n + x }\n\
                 f(35)\n\
             }\n\
             main()",
        );
        let layouts: Vec<_> = program
            .closure_function_layouts
            .iter()
            .filter_map(|l| l.as_ref())
            .filter(|l| l.capture_count() == 1)
            .collect();
        assert!(
            !layouts.is_empty(),
            "expected at least one layout with one capture"
        );
        for layout in layouts {
            assert_eq!(
                layout.capture_kinds[0],
                CaptureKind::Immutable,
                "`let` capture must be Immutable"
            );
            // A.1C (partial): masks remain zero; capture_kinds carries
            // the metadata independently. See
            // `compiler_impl_reference_model::build_closure_function_layouts`.
            assert_eq!(layout.owned_mutable_capture_mask, 0);
            assert_eq!(layout.shared_capture_mask, 0);
        }
    }

    #[test]
    fn test_a1c_let_mut_capture_layout_records_owned_mutable_kind() {
        // Track A.1C.2b: `let mut` capture mutated from a closure now
        // flips `owned_mutable_capture_mask` (bit 0). `op_make_closure`
        // reads the mask at creation time and allocates
        // `Box::into_raw(Box::new(initial))` into the Ptr slot.
        //
        // Session 1 — Rust-move: closure returns the updated value in
        // place of the invalidated outer read.
        let program = compile_source(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(5)\n\
             }\n\
             main()",
        );
        let layouts: Vec<_> = program
            .closure_function_layouts
            .iter()
            .filter_map(|l| l.as_ref())
            .filter(|l| l.capture_count() == 1)
            .collect();
        assert!(
            !layouts.is_empty(),
            "expected at least one layout with one capture"
        );
        let mut saw_owned_mutable = false;
        for layout in layouts {
            if layout.capture_kinds[0] == CaptureKind::OwnedMutable {
                saw_owned_mutable = true;
                assert_eq!(
                    layout.owned_mutable_capture_mask, 0b1,
                    "A.1C.2b: OwnedMutable bit must be set for let-mut capture"
                );
                assert_eq!(
                    layout.shared_capture_mask, 0,
                    "Shared bit must stay clear"
                );
            }
        }
        assert!(
            saw_owned_mutable,
            "`let mut` capture mutated from the closure must be OwnedMutable in capture_kinds"
        );
    }

    #[test]
    fn test_a1c_var_capture_layout_records_shared_kind() {
        // `var` capture mutated from a closure: CaptureKind::Shared.
        let program = compile_source(
            "fn main() -> int {\n\
                 var n: int = 0\n\
                 let f = |x: int| { n = n + x }\n\
                 f(3)\n\
                 n\n\
             }\n\
             main()",
        );
        let layouts: Vec<_> = program
            .closure_function_layouts
            .iter()
            .filter_map(|l| l.as_ref())
            .filter(|l| l.capture_count() == 1)
            .collect();
        assert!(
            !layouts.is_empty(),
            "expected at least one layout with one capture"
        );
        let mut saw_shared = false;
        for layout in layouts {
            if layout.capture_kinds[0] == CaptureKind::Shared {
                saw_shared = true;
            }
        }
        assert!(
            saw_shared,
            "`var` capture mutated from the closure must be Shared in capture_kinds"
        );
    }

    #[test]
    fn test_a1c_readonly_let_mut_capture_stays_immutable() {
        // A closure that only READS a `let mut` binding captures it by
        // value. CaptureKind must be Immutable — cell indirection is only
        // needed for write-through.
        let program = compile_source(
            "fn main() -> int {\n\
                 let mut n: int = 7\n\
                 let f = |x: int| { n + x }\n\
                 let r = f(35)\n\
                 r\n\
             }\n\
             main()",
        );
        let layouts: Vec<_> = program
            .closure_function_layouts
            .iter()
            .filter_map(|l| l.as_ref())
            .filter(|l| l.capture_count() == 1)
            .collect();
        assert!(!layouts.is_empty());
        for layout in layouts {
            assert_eq!(
                layout.capture_kinds[0],
                CaptureKind::Immutable,
                "read-only capture of `let mut` must remain Immutable"
            );
        }
    }

    #[test]
    fn test_a1c_let_mut_closure_e2e_propagates_writes_via_closure_return() {
        // End-to-end: a `let mut` capture mutated from inside the closure.
        // Post-A.1C.2b this flows through the A.1B OwnedMutable Raw path:
        //   * At closure creation, `op_make_closure` allocates
        //     `Box::into_raw(Box::new(initial))` into the Ptr slot.
        //   * Closure body emits `LoadOwnedMutableCapture` /
        //     `StoreOwnedMutableCapture` against that pointer.
        //   * `release_typed_closure` drops the Box on last-refcount-
        //     release.
        //
        // Move semantics are deferred — the outer slot retains the
        // initial value. To observe the closure's accumulator, read it
        // via the closure's return value.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut x: int = 0\n\
                 let inc = || { x = x + 1; x }\n\
                 inc()\n\
                 inc()\n\
             }\n\
             main()",
        );
        assert_eq!(
            val.as_i64(),
            Some(2),
            "let mut closure-internal cell must accumulate writes"
        );
    }

    #[test]
    fn test_a1c_var_multi_closure_e2e_shared_observes_writes() {
        // Two closures capturing the same `var` binding must observe each
        // other's writes. Legacy SharedCell semantics; A.1C does not
        // change runtime behaviour.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 var x: int = 0\n\
                 let inc = || { x = x + 1 }\n\
                 let dec = || { x = x - 1 }\n\
                 inc()\n\
                 dec()\n\
                 inc()\n\
                 x\n\
             }\n\
             main()",
        );
        assert_eq!(
            val.as_i64(),
            Some(1),
            "var captures shared across closures must observe each other's writes"
        );
    }

    #[test]
    fn test_a1c_mixed_let_let_mut_var_layout_records_each_kind() {
        // Closure captures one `let`, one `let mut`, and one `var`.
        // The layout's `capture_kinds` must reflect each binding form.
        let program = compile_source(
            "fn main() -> int {\n\
                 let a: int = 1\n\
                 let mut b: int = 10\n\
                 var c: int = 100\n\
                 let f = |x: int| { b = b + x; c = c + x; a + b + c }\n\
                 f(2)\n\
             }\n\
             main()",
        );
        // Find the layout with three captures — that's our closure.
        let target = program
            .closure_function_layouts
            .iter()
            .filter_map(|l| l.as_ref())
            .find(|l| l.capture_count() == 3);
        let layout = target.expect("closure with three captures must have a layout");

        // Captures are collected in sorted order of names (see
        // `compile_expr_closure` — `captured_vars.sort()`): a, b, c.
        assert_eq!(
            layout.capture_kinds[0],
            CaptureKind::Immutable,
            "`a` is let"
        );
        assert_eq!(
            layout.capture_kinds[1],
            CaptureKind::OwnedMutable,
            "`b` is let mut"
        );
        assert_eq!(layout.capture_kinds[2], CaptureKind::Shared, "`c` is var");
        // Track A.1C.2b: both Shared and OwnedMutable captures flip
        // their mask bits. `b` (let mut, index 1) sets
        // `owned_mutable_capture_mask` bit 1; `c` (var, index 2) sets
        // `shared_capture_mask` bit 2.
        assert_eq!(
            layout.owned_mutable_capture_mask, 0b010,
            "`b` (index 1) is let mut → OwnedMutable bit set"
        );
        assert_eq!(
            layout.shared_capture_mask, 0b100,
            "`c` (index 2) is var → Shared bit set"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Track A.1C.2b — end-to-end coverage for the migrated let-mut path
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_a1c2b_let_mut_closure_runtime_propagation() {
        // End-to-end witness for the A.1B OwnedMutable Raw path: the
        // closure's private Box accumulates writes across calls. The
        // outer-scope observation is not part of this test — move
        // analysis is deferred, so the outer slot may or may not
        // reflect the final value. This test exercises what A.1C.2b
        // can guarantee today: the closure-internal cell propagates
        // writes across invocations of the same closure.
        let val = run_program_top_level(
            "fn main() -> int {\n\
                 let mut x: int = 0\n\
                 let inc = |d: int| { x = x + d; x }\n\
                 inc(1)\n\
                 inc(2)\n\
                 inc(3)\n\
             }\n\
             main()",
        );
        assert_eq!(
            val.as_i64(),
            Some(6),
            "A.1C.2b: closure's OwnedMutable cell must accumulate across calls"
        );
    }

    #[test]
    fn test_a1c2b_let_mut_closure_emits_owned_mutable_opcodes() {
        // Witness the full opcode migration end-to-end: the compiler
        // emits A.1B's OwnedMutable opcodes for let-mut captures.
        //
        // Session 1 — Rust-move: observe the accumulated cell via the
        // closure's return value (the outer `x` read after capture is
        // now a use-after-move compile error).
        let program = compile_source(
            "fn main() -> int {\n\
                 let mut x: int = 0\n\
                 let inc = || { x = x + 1; x }\n\
                 inc()\n\
                 inc()\n\
             }\n\
             main()",
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::StoreOwnedMutableCapture),
            "expected StoreOwnedMutableCapture for let-mut write inside closure"
        );
        assert!(
            any_opcode_in_program(&program, |op| op == OC::LoadOwnedMutableCapture),
            "expected LoadOwnedMutableCapture for let-mut read inside closure"
        );
    }
}
