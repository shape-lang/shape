//! AST cloning + type substitution for generic monomorphization.
//!
//! When the compiler encounters a generic call like `arr.map(|x| x + 1.0)`
//! where `arr: Array<number>`, it needs to produce a specialized
//! `FunctionDef` for `map<number, number>`. That specialized def must have
//! every `T` (and other type params) replaced by the resolved
//! `ConcreteType`, both in parameter / return annotations *and* in any type
//! annotations sprinkled through the body (`let x: T = ...`,
//! `let arr: Array<T> = ...`, etc).
//!
//! Pipeline contract:
//!
//! 1. Sibling [`super::type_resolution`] (Agent 1) computes
//!    `HashMap<String, ConcreteType>` from a call site (e.g.
//!    `T -> i64`, `U -> string`).
//! 2. We deep-clone the `FunctionDef` and walk every `TypeAnnotation` inside
//!    it. Anywhere a `Reference(path)` matches one of the type parameter
//!    names, we replace it with the corresponding `ConcreteType` (rendered
//!    back into `TypeAnnotation` form via [`concrete_to_annotation`]).
//! 3. The cloned function gets a uniquified name (`"map::f64_string"`) so
//!    [`super::cache::MonomorphizationCache`] (Agent 3) can hand back the
//!    same compiled body for repeated callsites.
//!
//! Soundness note: we substitute `Reference(path)` only when the path is
//! single-segment. That is correct because Shape's generics use bare
//! identifiers (`T`, `U`, etc.) — see `TypeParam.name` in
//! `shape-ast::ast::types`. A qualified reference like `mod::T` is not a
//! type parameter and is intentionally left untouched.

use shape_ast::ast::expr_helpers::{
    AssignExpr, AsyncLetExpr, BlockExpr, BlockItem, ComprehensionClause, ComptimeForExpr, ForExpr,
    FromQueryExpr, IfExpr, JoinBranch, JoinExpr, LetExpr, ListComprehension, LoopExpr, MatchArm,
    MatchExpr, QueryClause, WhileExpr,
};
use shape_ast::ast::expressions::{EnumConstructorPayload, Expr, ObjectEntry};
use shape_ast::ast::functions::{FunctionDef, FunctionParameter};
use shape_ast::ast::patterns::{
    DecompositionBinding, DestructurePattern, ObjectPatternField, Pattern, PatternConstructorFields,
};
use shape_ast::ast::statements::{ForInit, IfStatement, Statement, WhileLoop};
use shape_ast::ast::type_path::TypePath;
use shape_ast::ast::types::{
    ExtendStatement, FunctionParam, MethodDef, ObjectTypeField, TypeAnnotation,
};
use shape_value::v2::ConcreteType;
use std::collections::HashMap;

use crate::compiler::monomorphization::type_resolution::ComptimeConstValue;

// ---------------------------------------------------------------------------
// Legacy placeholder symbol — kept so the meta integration test in
// `integration_tests.rs` (Agent 4) keeps compiling unchanged.
//
// Once the real substitution path is wired into `BytecodeCompiler` the
// integration meta-test can drop this reference. Until then, we expose the
// constant with the same name and a non-empty value.
// ---------------------------------------------------------------------------

/// Historical sentinel used by Agent 4's meta integration test as proof that
/// the `monomorphization::substitution` module is reachable. Keeping the
/// constant avoids invalidating Agent 4's `test_monomorphization_module_exists`
/// while the real substitution implementation lives behind the same module
/// path.
pub const SUBSTITUTION_NOT_INTEGRATED: &str = "monomorphization::substitution is integrated";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a `ConcreteType` back into the AST `TypeAnnotation` form so the
/// substituted function looks (to the rest of the compiler) like a hand-
/// written monomorphic function. The naming convention here mirrors the
/// stdlib's user-visible spelling: `F64` → `"number"`, `I64` → `"int"`.
pub fn concrete_to_annotation(ct: &ConcreteType) -> TypeAnnotation {
    match ct {
        // Primitive scalar mapping. Width-specific integers use the spelling
        // accepted by the parser (see `shape.pest` and `BuiltinTypes`).
        ConcreteType::F64 => TypeAnnotation::Basic("number".into()),
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment.
        ConcreteType::F32 => TypeAnnotation::Basic("f32".into()),
        ConcreteType::Char => TypeAnnotation::Basic("char".into()),
        ConcreteType::I64 => TypeAnnotation::Basic("int".into()),
        ConcreteType::I32 => TypeAnnotation::Basic("i32".into()),
        ConcreteType::I16 => TypeAnnotation::Basic("i16".into()),
        ConcreteType::I8 => TypeAnnotation::Basic("i8".into()),
        ConcreteType::U64 => TypeAnnotation::Basic("u64".into()),
        ConcreteType::U32 => TypeAnnotation::Basic("u32".into()),
        ConcreteType::U16 => TypeAnnotation::Basic("u16".into()),
        ConcreteType::U8 => TypeAnnotation::Basic("u8".into()),
        ConcreteType::Bool => TypeAnnotation::Basic("bool".into()),
        ConcreteType::String => TypeAnnotation::Basic("string".into()),
        ConcreteType::Decimal => TypeAnnotation::Basic("decimal".into()),
        ConcreteType::BigInt => TypeAnnotation::Basic("bigint".into()),
        ConcreteType::DateTime => TypeAnnotation::Basic("DateTime".into()),

        // Container/composite types — re-emit as `Generic { name, args }`
        // so downstream type inference treats them like the user-written
        // form.
        ConcreteType::Array(elem) => TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![concrete_to_annotation(elem)],
        },
        ConcreteType::HashMap(k, v) => TypeAnnotation::Generic {
            name: TypePath::simple("HashMap"),
            args: vec![concrete_to_annotation(k), concrete_to_annotation(v)],
        },
        ConcreteType::Option(inner) => TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![concrete_to_annotation(inner)],
        },
        ConcreteType::Result(ok, err) => TypeAnnotation::Generic {
            name: TypePath::simple("Result"),
            args: vec![concrete_to_annotation(ok), concrete_to_annotation(err)],
        },
        ConcreteType::Tuple(elems) => {
            TypeAnnotation::Tuple(elems.iter().map(concrete_to_annotation).collect())
        }

        ConcreteType::Pointer(inner) => TypeAnnotation::Generic {
            name: TypePath::simple("ptr"),
            args: vec![concrete_to_annotation(inner)],
        },

        // Opaque IDs: there is no source-level spelling for "the struct with
        // layout #4", so we synthesize a unique reference name. The
        // monomorphization cache uses `mono_key()` (which is bijective on
        // these IDs) so it never collides with a user identifier.
        ConcreteType::Struct(id) => {
            TypeAnnotation::Reference(TypePath::simple(format!("__mono_struct_{}", id.0)))
        }
        ConcreteType::Enum(id) => {
            TypeAnnotation::Reference(TypePath::simple(format!("__mono_enum_{}", id.0)))
        }
        ConcreteType::Closure(id) => {
            TypeAnnotation::Reference(TypePath::simple(format!("__mono_closure_{}", id.0)))
        }
        ConcreteType::Function(id) => {
            TypeAnnotation::Reference(TypePath::simple(format!("__mono_fn_{}", id.0)))
        }

        // ── Phase 3 cluster-0 Round 11-trinity 11E (2026-05-13) ─────────
        // Collection / concurrency carriers from ADR-006 §2.7.15 /
        // §2.7.17 / §2.7.18 / §2.7.20 / §2.7.25 round-trip back to the
        // user-written `Generic { name, args }` form for downstream type
        // inference. Parametric arms unwrap their inner ConcreteType
        // recursively; nullary arms (`PriorityQueue`, `Atomic`) map to
        // the no-args reference form — same shape as a user-written
        // `PriorityQueue` / `Atomic` identifier in source.
        ConcreteType::HashSet(elem) => TypeAnnotation::Generic {
            name: TypePath::simple("HashSet"),
            args: vec![concrete_to_annotation(elem)],
        },
        ConcreteType::Deque(elem) => TypeAnnotation::Generic {
            name: TypePath::simple("Deque"),
            args: vec![concrete_to_annotation(elem)],
        },
        ConcreteType::PriorityQueue => {
            TypeAnnotation::Reference(TypePath::simple("PriorityQueue"))
        }
        ConcreteType::Channel(elem) => TypeAnnotation::Generic {
            name: TypePath::simple("Channel"),
            args: vec![concrete_to_annotation(elem)],
        },
        ConcreteType::Mutex(inner) => TypeAnnotation::Generic {
            name: TypePath::simple("Mutex"),
            args: vec![concrete_to_annotation(inner)],
        },
        ConcreteType::Atomic => TypeAnnotation::Reference(TypePath::simple("Atomic")),
        ConcreteType::Lazy(inner) => TypeAnnotation::Generic {
            name: TypePath::simple("Lazy"),
            args: vec![concrete_to_annotation(inner)],
        },

        ConcreteType::Void => TypeAnnotation::Void,
    }
}

/// Recursively walk a `TypeAnnotation`, replacing every type-parameter
/// reference with its concrete substitution. Annotations that don't mention
/// any of the substitution keys are returned structurally cloned.
pub fn substitute_type_annotation(
    ann: &TypeAnnotation,
    subs: &HashMap<String, ConcreteType>,
) -> TypeAnnotation {
    match ann {
        // The interesting case: a bare type-parameter reference like `T`.
        // Only substitute when the path is single-segment (i.e. a plain
        // identifier, not a qualified name) and matches a type parameter.
        TypeAnnotation::Reference(path) => {
            if !path.is_qualified() {
                if let Some(ct) = subs.get(path.as_str()) {
                    return concrete_to_annotation(ct);
                }
            }
            TypeAnnotation::Reference(path.clone())
        }
        // Some Shape source uses `Basic("T")` for unqualified type names —
        // the parser collapses single identifiers depending on context.
        // Treat them the same way for safety.
        TypeAnnotation::Basic(name) => {
            if let Some(ct) = subs.get(name) {
                concrete_to_annotation(ct)
            } else {
                TypeAnnotation::Basic(name.clone())
            }
        }

        TypeAnnotation::Array(inner) => {
            TypeAnnotation::Array(Box::new(substitute_type_annotation(inner, subs)))
        }

        TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
            items
                .iter()
                .map(|t| substitute_type_annotation(t, subs))
                .collect(),
        ),

        TypeAnnotation::Object(fields) => TypeAnnotation::Object(
            fields
                .iter()
                .map(|f| ObjectTypeField {
                    name: f.name.clone(),
                    optional: f.optional,
                    type_annotation: substitute_type_annotation(&f.type_annotation, subs),
                    annotations: f.annotations.clone(),
                })
                .collect(),
        ),

        TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
            params: params
                .iter()
                .map(|p| FunctionParam {
                    name: p.name.clone(),
                    optional: p.optional,
                    type_annotation: substitute_type_annotation(&p.type_annotation, subs),
                })
                .collect(),
            returns: Box::new(substitute_type_annotation(returns, subs)),
        },

        TypeAnnotation::Union(items) => TypeAnnotation::Union(
            items
                .iter()
                .map(|t| substitute_type_annotation(t, subs))
                .collect(),
        ),

        TypeAnnotation::Intersection(items) => TypeAnnotation::Intersection(
            items
                .iter()
                .map(|t| substitute_type_annotation(t, subs))
                .collect(),
        ),

        // Generic types: substitute through the type arguments. The name
        // itself isn't a type parameter (the grammar forbids `T<U>` for a
        // generic param), so it stays as-is.
        TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_type_annotation(a, subs))
                .collect(),
        },

        // Leaves with no nested type annotations.
        TypeAnnotation::Void => TypeAnnotation::Void,
        TypeAnnotation::Never => TypeAnnotation::Never,
        TypeAnnotation::Null => TypeAnnotation::Null,
        TypeAnnotation::Undefined => TypeAnnotation::Undefined,
        TypeAnnotation::Dyn(paths) => TypeAnnotation::Dyn(paths.clone()),
    }
}

/// Build the deterministic suffix used in specialization keys / function
/// names: e.g. `{T -> i64, U -> string}` → `"i64_string"`. Sorting by
/// type-parameter name guarantees stable hashing for the cache.
pub fn mono_key_from_subs(subs: &HashMap<String, ConcreteType>) -> String {
    let mut entries: Vec<(&String, &ConcreteType)> = subs.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
        .iter()
        .map(|(_, ct)| ct.mono_key())
        .collect::<Vec<_>>()
        .join("_")
}

/// Deep-clone `def` and substitute all type parameters. The returned
/// `FunctionDef` has:
///
/// - Substituted parameter `type_annotation`s and `default_value`s walked.
/// - Substituted `return_type`.
/// - Substituted type annotations everywhere in the body (`let x: T`,
///   `let arr: Array<T>`, type assertions, lambda return types, etc).
/// - A unique name with the mono key suffix appended (`"id::i64"`).
/// - The `type_params` list cleared — the function is now fully concrete.
pub fn substitute_function_def(
    def: &FunctionDef,
    subs: &HashMap<String, ConcreteType>,
) -> FunctionDef {
    let mut cloned = def.clone();

    // Substitute parameters (annotation + any default value expressions).
    cloned.params = def
        .params
        .iter()
        .map(|p| substitute_function_parameter(p, subs))
        .collect();

    // Substitute return type.
    cloned.return_type = def
        .return_type
        .as_ref()
        .map(|t| substitute_type_annotation(t, subs));

    // Walk the body for nested type annotations.
    cloned.body = def
        .body
        .iter()
        .map(|s| substitute_statement(s, subs))
        .collect();

    // V3-S6a resolver-extension follow-up: empty-array terminal-result
    // annotation synthesis. The generic source `let mut result = []` carries
    // no annotation because `Array<U>` can't be spelled until U binds. After
    // substitution rewrites the function's return type to a concrete
    // `Array<C>`, the empty literal still lacks the kind hint the typed-
    // array emission path needs (`pending_variable_typed_array_kind`).
    //
    // Synthesize the annotation for the shape that matches the
    // `Vec.map<U>` / `Vec.filter` body pattern: terminal expression is an
    // identifier referring to a `let mut <name> = []` declared earlier
    // with no annotation, and the function returns `Array<C>` for some
    // concrete C. This is structurally narrow (it only fires on the
    // exact shape produced by collect-into-array stdlib bodies) and
    // architecturally honest: monomorphization synthesizes annotations
    // the generic source couldn't express.
    synthesize_empty_array_result_annotation(&mut cloned);

    // Rename so the specialization cache can key on the new name.
    cloned.name = format!("{}::{}", def.name, mono_key_from_subs(subs));

    // The cloned function is now fully concrete; drop generics so the rest
    // of the pipeline doesn't try to re-instantiate it.
    cloned.type_params = None;

    cloned
}

/// V3-S6a resolver-extension follow-up: after monomorphization substitution
/// concretizes the function's return type, walk the body for the canonical
/// "collect-into-array" shape and annotate the empty-array initializer so
/// the bytecode compiler's typed-array emission path picks up the right
/// element kind via `pending_variable_typed_array_kind`.
///
/// Scope: the function's return type is `Array<C>` for some concrete C
/// (post-substitution, no remaining type-parameter references), AND the
/// body's terminal expression is `Identifier(name)`, AND there exists a
/// `Statement::VariableDecl` with `pattern == Identifier(name)`,
/// `type_annotation == None`, and `value == Some(Expr::Array(<empty>))`.
/// In that case, write `Array<C>` onto that var-decl's annotation.
///
/// Out of scope (intentionally):
/// - Non-Array return types (no typed-array kind to propagate).
/// - Bodies whose terminal expression is not a bare identifier (a `Block`
///   ending in `result`, an explicit `return result`, etc — those land in
///   different ckpts if they regress, kept narrow here per cascade-ceiling
///   discipline).
/// - Var-decls whose `type_annotation` is already set (the user wrote it
///   — trust it).
fn synthesize_empty_array_result_annotation(def: &mut FunctionDef) {
    // Step 1: read the concrete element type out of the return annotation,
    // if it is `Array<C>`. Bail otherwise.
    let elem_annotation: TypeAnnotation = match def.return_type.as_ref() {
        Some(TypeAnnotation::Generic { name, args }) if name.as_str() == "Array" && args.len() == 1 => {
            args[0].clone()
        }
        Some(TypeAnnotation::Generic { name, args }) if name.as_str() == "Vec" && args.len() == 1 => {
            args[0].clone()
        }
        Some(TypeAnnotation::Array(inner)) => (**inner).clone(),
        _ => return,
    };

    // Step 2: find the body's terminal expression. The Vec.map / Vec.filter
    // pattern leaves `result` as the last `Statement::Expression`. Bail if
    // the body's last statement isn't an `Expression(Identifier)`.
    let Some(Statement::Expression(Expr::Identifier(terminal_name, _), _)) = def.body.last() else {
        return;
    };
    let terminal_name = terminal_name.clone();

    // Step 3: find the matching `let mut <name> = []` var-decl earlier in
    // the body. Mutate in place when found.
    for stmt in def.body.iter_mut() {
        let Statement::VariableDecl(decl, _) = stmt else {
            continue;
        };
        // Only rewrite the var-decl whose identifier matches the terminal.
        let Some(decl_name) = decl.pattern.as_identifier() else {
            continue;
        };
        if decl_name != terminal_name {
            continue;
        }
        // Don't override a user-written annotation.
        if decl.type_annotation.is_some() {
            continue;
        }
        // Initializer must be an empty array literal.
        let is_empty_array = matches!(
            decl.value.as_ref(),
            Some(Expr::Array(items, _)) if items.is_empty()
        );
        if !is_empty_array {
            continue;
        }
        // Synthesize `Array<C>` annotation. The bytecode compiler's
        // `pending_variable_typed_array_kind` path reads this annotation
        // and routes the empty literal through the typed-array opcode
        // selection in `compile_expr_array`.
        decl.type_annotation = Some(TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![elem_annotation.clone()],
        });
        return;
    }
}

/// Const-generic-aware variant of [`substitute_function_def`].
///
/// Behaves identically to [`substitute_function_def`] for the type-substitution
/// pass, but ALSO substitutes const generic parameter references throughout
/// the body. The cloned function is renamed using the caller-supplied
/// `mono_key` (which already encodes both the type args and the const args
/// via [`crate::compiler::monomorphization::type_resolution::build_mono_key_with_consts`])
/// so the specialization cache stays consistent across the type-only and
/// type+const code paths.
///
/// Const substitution rules:
///
///   - Any `Reference(N)` / `Basic(N)` annotation whose name matches a key in
///     `const_subs` is replaced with a synthetic `__const_<value>` reference.
///     Today this is purely a uniquification trick — the bytecode compiler
///     never lowers it to anything because the grammar doesn't allow const
///     generic params to appear in type positions yet. The path is here so
///     when the grammar lands the substitution surface is already in place.
///
///   - Any `Identifier(N)` expression in the body whose name matches a const
///     generic name is rewritten to a literal expression with the bound
///     value. This is the path that turns `repeat<3>` into `repeat::int_3`
///     where the body literally sees `3` for `N`.
///
/// **Grammar gap**: see the audit notes at the top of
/// [`crate::compiler::monomorphization::type_resolution`]. Until the grammar
/// allows `const N: int` in `<...>` positions, the only way to exercise this
/// path is via the unit tests in this module which build `FunctionDef`
/// instances by hand.
pub fn substitute_function_def_with_consts(
    def: &FunctionDef,
    type_subs: &HashMap<String, ConcreteType>,
    const_subs: &HashMap<String, ComptimeConstValue>,
    mono_key: &str,
) -> FunctionDef {
    let mut cloned = def.clone();

    // Substitute parameter annotations and default values, walking both the
    // type-substitution and const-substitution maps.
    cloned.params = def
        .params
        .iter()
        .map(|p| {
            let mut np = substitute_function_parameter(p, type_subs);
            np.default_value = p
                .default_value
                .as_ref()
                .map(|e| substitute_const_in_expr(e, const_subs));
            np
        })
        .collect();

    cloned.return_type = def
        .return_type
        .as_ref()
        .map(|t| substitute_type_annotation(t, type_subs));

    // Walk the body for both type-annotation substitution and const-name
    // expression substitution. The two passes commute (types and exprs live
    // in disjoint AST positions) so order doesn't matter.
    cloned.body = def
        .body
        .iter()
        .map(|s| {
            let s = substitute_statement(s, type_subs);
            substitute_const_in_statement(&s, const_subs)
        })
        .collect();

    // V3-S6a resolver-extension follow-up: lift the empty-array terminal-
    // result annotation synthesis here too (same shape as in the non-const
    // path). See `synthesize_empty_array_result_annotation`.
    synthesize_empty_array_result_annotation(&mut cloned);

    // Use the caller-supplied mono_key directly so this stays in lock-step
    // with `build_mono_key_with_consts`.
    cloned.name = format!("{}::{}", def.name, strip_fn_name_prefix(&def.name, mono_key));

    // The cloned function is now fully concrete; drop generics so the rest
    // of the pipeline doesn't try to re-instantiate it.
    cloned.type_params = None;

    cloned
}

/// Strip the leading `<fn_name>::` from a mono_key, returning just the
/// suffix. Used so the new function name follows the same `name::suffix`
/// convention as [`substitute_function_def`] without double-prefixing.
fn strip_fn_name_prefix<'a>(fn_name: &str, mono_key: &'a str) -> &'a str {
    let prefix = format!("{}::", fn_name);
    mono_key.strip_prefix(&prefix).unwrap_or(mono_key)
}

/// Walk a statement and rewrite any identifier expression whose name matches
/// a const generic param into a literal expression with the bound value.
///
/// Type annotations are NOT walked here — they were already handled in the
/// preceding type-substitution pass. This pass only touches expression nodes.
///
/// **Coverage**: exhaustive across every `Statement` variant. The compiler's
/// exhaustive-match rule (see CLAUDE.md) ensures any future variant addition
/// forces a corresponding arm here. Leaf statements (`Break`, `Continue`,
/// `RemoveTarget`) are returned unchanged; every statement that holds an
/// `Expr` or nested `Statement` recurses into its sub-AST.
fn substitute_const_in_statement(
    stmt: &Statement,
    const_subs: &HashMap<String, ComptimeConstValue>,
) -> Statement {
    if const_subs.is_empty() {
        return stmt.clone();
    }
    match stmt {
        Statement::Return(expr, span) => Statement::Return(
            expr.as_ref()
                .map(|e| substitute_const_in_expr(e, const_subs)),
            *span,
        ),
        Statement::Break(span) => Statement::Break(*span),
        Statement::Continue(span) => Statement::Continue(*span),

        Statement::VariableDecl(decl, span) => {
            let mut new_decl = decl.clone();
            new_decl.value = decl
                .value
                .as_ref()
                .map(|e| substitute_const_in_expr(e, const_subs));
            Statement::VariableDecl(new_decl, *span)
        }

        Statement::Assignment(assign, span) => {
            let mut new_assign = assign.clone();
            new_assign.value = substitute_const_in_expr(&assign.value, const_subs);
            Statement::Assignment(new_assign, *span)
        }

        Statement::Expression(expr, span) => {
            Statement::Expression(substitute_const_in_expr(expr, const_subs), *span)
        }

        Statement::For(for_loop, span) => {
            let mut new_loop = for_loop.clone();
            new_loop.init = match &for_loop.init {
                ForInit::ForIn { pattern, iter } => ForInit::ForIn {
                    pattern: pattern.clone(),
                    iter: substitute_const_in_expr(iter, const_subs),
                },
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => ForInit::ForC {
                    init: Box::new(substitute_const_in_statement(init, const_subs)),
                    condition: substitute_const_in_expr(condition, const_subs),
                    update: substitute_const_in_expr(update, const_subs),
                },
            };
            new_loop.body = for_loop
                .body
                .iter()
                .map(|s| substitute_const_in_statement(s, const_subs))
                .collect();
            Statement::For(new_loop, *span)
        }

        Statement::While(while_loop, span) => Statement::While(
            WhileLoop {
                condition: substitute_const_in_expr(&while_loop.condition, const_subs),
                body: while_loop
                    .body
                    .iter()
                    .map(|s| substitute_const_in_statement(s, const_subs))
                    .collect(),
            },
            *span,
        ),

        Statement::If(if_stmt, span) => Statement::If(
            IfStatement {
                condition: substitute_const_in_expr(&if_stmt.condition, const_subs),
                then_body: if_stmt
                    .then_body
                    .iter()
                    .map(|s| substitute_const_in_statement(s, const_subs))
                    .collect(),
                else_body: if_stmt.else_body.as_ref().map(|body| {
                    body.iter()
                        .map(|s| substitute_const_in_statement(s, const_subs))
                        .collect()
                }),
            },
            *span,
        ),

        // `extend` blocks hold method defs whose bodies could reference a
        // const generic param in scope. Walk each method body.
        Statement::Extend(ext, span) => {
            let mut new_ext = ext.clone();
            new_ext.methods = ext
                .methods
                .iter()
                .map(|m| {
                    let mut cm = m.clone();
                    cm.body = m
                        .body
                        .iter()
                        .map(|s| substitute_const_in_statement(s, const_subs))
                        .collect();
                    cm.when_clause = m
                        .when_clause
                        .as_ref()
                        .map(|e| Box::new(substitute_const_in_expr(e, const_subs)));
                    cm
                })
                .collect();
            Statement::Extend(new_ext, *span)
        }

        Statement::RemoveTarget(span) => Statement::RemoveTarget(*span),

        // SetParamType / SetReturnType carry only a TypeAnnotation — the
        // type-subst pass handled it; nothing expression-shaped to rewrite.
        Statement::SetParamType {
            param_name,
            type_annotation,
            span,
        } => Statement::SetParamType {
            param_name: param_name.clone(),
            type_annotation: type_annotation.clone(),
            span: *span,
        },

        Statement::SetParamValue {
            param_name,
            expression,
            span,
        } => Statement::SetParamValue {
            param_name: param_name.clone(),
            expression: substitute_const_in_expr(expression, const_subs),
            span: *span,
        },

        Statement::SetReturnType {
            type_annotation,
            span,
        } => Statement::SetReturnType {
            type_annotation: type_annotation.clone(),
            span: *span,
        },

        Statement::SetReturnExpr { expression, span } => Statement::SetReturnExpr {
            expression: substitute_const_in_expr(expression, const_subs),
            span: *span,
        },

        Statement::ReplaceBody { body, span } => Statement::ReplaceBody {
            body: body
                .iter()
                .map(|s| substitute_const_in_statement(s, const_subs))
                .collect(),
            span: *span,
        },

        Statement::ReplaceBodyExpr { expression, span } => Statement::ReplaceBodyExpr {
            expression: substitute_const_in_expr(expression, const_subs),
            span: *span,
        },

        Statement::ReplaceModuleExpr { expression, span } => Statement::ReplaceModuleExpr {
            expression: substitute_const_in_expr(expression, const_subs),
            span: *span,
        },
    }
}

/// Recursively rewrite identifier expressions to literals when they bind to
/// a const generic parameter. Walks every `Expr` variant structurally so an
/// identifier embedded anywhere (method-call arg, match arm body, closure
/// body, etc.) gets replaced.
///
/// The exhaustive match mirrors [`substitute_expr`] — adding a new `Expr`
/// variant to the AST forces a compile error here, driving the CLAUDE.md
/// "Exhaustive Match Rule" guarantee.
fn substitute_const_in_expr(
    expr: &Expr,
    const_subs: &HashMap<String, ComptimeConstValue>,
) -> Expr {
    if const_subs.is_empty() {
        return expr.clone();
    }
    match expr {
        Expr::Identifier(name, span) => {
            if let Some(value) = const_subs.get(name) {
                if let Some(lit) = const_value_to_literal(value, *span) {
                    return lit;
                }
            }
            expr.clone()
        }

        // Leaves with no sub-expressions.
        Expr::Literal(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::Duration(_, _)
        | Expr::Continue(_)
        | Expr::Unit(_)
        | Expr::TableRows(_, _) => expr.clone(),

        Expr::DataRelativeAccess {
            reference,
            index,
            span,
        } => Expr::DataRelativeAccess {
            reference: Box::new(substitute_const_in_expr(reference, const_subs)),
            index: index.clone(),
            span: *span,
        },

        Expr::PropertyAccess {
            object,
            property,
            optional,
            span,
        } => Expr::PropertyAccess {
            object: Box::new(substitute_const_in_expr(object, const_subs)),
            property: property.clone(),
            optional: *optional,
            span: *span,
        },

        Expr::IndexAccess {
            object,
            index,
            end_index,
            span,
        } => Expr::IndexAccess {
            object: Box::new(substitute_const_in_expr(object, const_subs)),
            index: Box::new(substitute_const_in_expr(index, const_subs)),
            end_index: end_index
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            span: *span,
        },

        Expr::BinaryOp {
            left,
            op,
            right,
            span,
        } => Expr::BinaryOp {
            left: Box::new(substitute_const_in_expr(left, const_subs)),
            op: op.clone(),
            right: Box::new(substitute_const_in_expr(right, const_subs)),
            span: *span,
        },

        Expr::FuzzyComparison {
            left,
            op,
            right,
            tolerance,
            span,
        } => Expr::FuzzyComparison {
            left: Box::new(substitute_const_in_expr(left, const_subs)),
            op: op.clone(),
            right: Box::new(substitute_const_in_expr(right, const_subs)),
            tolerance: tolerance.clone(),
            span: *span,
        },

        Expr::UnaryOp { op, operand, span } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(substitute_const_in_expr(operand, const_subs)),
            span: *span,
        },

        Expr::FunctionCall {
            name,
            args,
            named_args,
            span,
        } => Expr::FunctionCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_const_in_expr(a, const_subs))
                .collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                .collect(),
            span: *span,
        },

        Expr::QualifiedFunctionCall {
            namespace,
            function,
            args,
            named_args,
            span,
        } => Expr::QualifiedFunctionCall {
            namespace: namespace.clone(),
            function: function.clone(),
            args: args
                .iter()
                .map(|a| substitute_const_in_expr(a, const_subs))
                .collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                .collect(),
            span: *span,
        },

        Expr::EnumConstructor {
            enum_name,
            variant,
            payload,
            span,
        } => Expr::EnumConstructor {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: match payload {
                EnumConstructorPayload::Unit => EnumConstructorPayload::Unit,
                EnumConstructorPayload::Tuple(args) => EnumConstructorPayload::Tuple(
                    args.iter()
                        .map(|a| substitute_const_in_expr(a, const_subs))
                        .collect(),
                ),
                EnumConstructorPayload::Struct(fields) => EnumConstructorPayload::Struct(
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                        .collect(),
                ),
            },
            span: *span,
        },

        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            span,
        } => Expr::Conditional {
            condition: Box::new(substitute_const_in_expr(condition, const_subs)),
            then_expr: Box::new(substitute_const_in_expr(then_expr, const_subs)),
            else_expr: else_expr
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            span: *span,
        },

        Expr::Object(entries, span) => Expr::Object(
            entries
                .iter()
                .map(|e| match e {
                    ObjectEntry::Field {
                        key,
                        value,
                        type_annotation,
                    } => ObjectEntry::Field {
                        key: key.clone(),
                        value: substitute_const_in_expr(value, const_subs),
                        type_annotation: type_annotation.clone(),
                    },
                    ObjectEntry::Spread(inner) => {
                        ObjectEntry::Spread(substitute_const_in_expr(inner, const_subs))
                    }
                })
                .collect(),
            *span,
        ),

        Expr::Array(items, span) => Expr::Array(
            items
                .iter()
                .map(|i| substitute_const_in_expr(i, const_subs))
                .collect(),
            *span,
        ),

        Expr::ListComprehension(comp, span) => Expr::ListComprehension(
            Box::new(ListComprehension {
                element: Box::new(substitute_const_in_expr(&comp.element, const_subs)),
                clauses: comp
                    .clauses
                    .iter()
                    .map(|c| ComprehensionClause {
                        pattern: c.pattern.clone(),
                        iterable: Box::new(substitute_const_in_expr(&c.iterable, const_subs)),
                        filter: c
                            .filter
                            .as_ref()
                            .map(|f| Box::new(substitute_const_in_expr(f, const_subs))),
                    })
                    .collect(),
            }),
            *span,
        ),

        Expr::Block(block, span) => {
            let new_items = block
                .items
                .iter()
                .map(|item| match item {
                    BlockItem::VariableDecl(decl) => {
                        let mut new_decl = decl.clone();
                        new_decl.value = decl
                            .value
                            .as_ref()
                            .map(|e| substitute_const_in_expr(e, const_subs));
                        BlockItem::VariableDecl(new_decl)
                    }
                    BlockItem::Assignment(assign) => {
                        let mut new_assign = assign.clone();
                        new_assign.value = substitute_const_in_expr(&assign.value, const_subs);
                        BlockItem::Assignment(new_assign)
                    }
                    BlockItem::Statement(s) => {
                        BlockItem::Statement(substitute_const_in_statement(s, const_subs))
                    }
                    BlockItem::Expression(e) => {
                        BlockItem::Expression(substitute_const_in_expr(e, const_subs))
                    }
                })
                .collect();
            Expr::Block(BlockExpr { items: new_items }, *span)
        }

        Expr::TypeAssertion {
            expr,
            type_annotation,
            meta_param_overrides,
            span,
        } => Expr::TypeAssertion {
            expr: Box::new(substitute_const_in_expr(expr, const_subs)),
            type_annotation: type_annotation.clone(),
            meta_param_overrides: meta_param_overrides.clone(),
            span: *span,
        },

        Expr::InstanceOf {
            expr,
            type_annotation,
            span,
        } => Expr::InstanceOf {
            expr: Box::new(substitute_const_in_expr(expr, const_subs)),
            type_annotation: type_annotation.clone(),
            span: *span,
        },

        Expr::FunctionExpr {
            params,
            return_type,
            body,
            span,
        } => Expr::FunctionExpr {
            // Closure params may have default-value exprs that reference
            // const generics; walk those but leave patterns/annotations alone.
            params: params
                .iter()
                .map(|p| {
                    let mut np = p.clone();
                    np.default_value = p
                        .default_value
                        .as_ref()
                        .map(|e| substitute_const_in_expr(e, const_subs));
                    np
                })
                .collect(),
            return_type: return_type.clone(),
            body: body
                .iter()
                .map(|s| substitute_const_in_statement(s, const_subs))
                .collect(),
            span: *span,
        },

        Expr::Spread(inner, span) => {
            Expr::Spread(Box::new(substitute_const_in_expr(inner, const_subs)), *span)
        }

        Expr::If(if_expr, span) => Expr::If(
            Box::new(IfExpr {
                condition: Box::new(substitute_const_in_expr(&if_expr.condition, const_subs)),
                then_branch: Box::new(substitute_const_in_expr(&if_expr.then_branch, const_subs)),
                else_branch: if_expr
                    .else_branch
                    .as_ref()
                    .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            }),
            *span,
        ),

        Expr::While(while_expr, span) => Expr::While(
            Box::new(WhileExpr {
                condition: Box::new(substitute_const_in_expr(&while_expr.condition, const_subs)),
                body: Box::new(substitute_const_in_expr(&while_expr.body, const_subs)),
            }),
            *span,
        ),

        Expr::For(for_expr, span) => Expr::For(
            Box::new(ForExpr {
                pattern: for_expr.pattern.clone(),
                iterable: Box::new(substitute_const_in_expr(&for_expr.iterable, const_subs)),
                body: Box::new(substitute_const_in_expr(&for_expr.body, const_subs)),
                is_async: for_expr.is_async,
            }),
            *span,
        ),

        Expr::Loop(loop_expr, span) => Expr::Loop(
            Box::new(LoopExpr {
                body: Box::new(substitute_const_in_expr(&loop_expr.body, const_subs)),
            }),
            *span,
        ),

        Expr::Let(let_expr, span) => Expr::Let(
            Box::new(LetExpr {
                pattern: let_expr.pattern.clone(),
                type_annotation: let_expr.type_annotation.clone(),
                value: let_expr
                    .value
                    .as_ref()
                    .map(|v| Box::new(substitute_const_in_expr(v, const_subs))),
                body: Box::new(substitute_const_in_expr(&let_expr.body, const_subs)),
            }),
            *span,
        ),

        Expr::Assign(assign_expr, span) => Expr::Assign(
            Box::new(AssignExpr {
                target: Box::new(substitute_const_in_expr(&assign_expr.target, const_subs)),
                value: Box::new(substitute_const_in_expr(&assign_expr.value, const_subs)),
            }),
            *span,
        ),

        Expr::Break(value, span) => Expr::Break(
            value
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            *span,
        ),

        Expr::Return(value, span) => Expr::Return(
            value
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            *span,
        ),

        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            optional,
            span,
        } => Expr::MethodCall {
            receiver: Box::new(substitute_const_in_expr(receiver, const_subs)),
            method: method.clone(),
            args: args
                .iter()
                .map(|a| substitute_const_in_expr(a, const_subs))
                .collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                .collect(),
            optional: *optional,
            span: *span,
        },

        Expr::Match(match_expr, span) => Expr::Match(
            Box::new(MatchExpr {
                scrutinee: Box::new(substitute_const_in_expr(&match_expr.scrutinee, const_subs)),
                arms: match_expr
                    .arms
                    .iter()
                    .map(|arm| MatchArm {
                        pattern: arm.pattern.clone(),
                        guard: arm
                            .guard
                            .as_ref()
                            .map(|g| Box::new(substitute_const_in_expr(g, const_subs))),
                        body: Box::new(substitute_const_in_expr(&arm.body, const_subs)),
                        pattern_span: arm.pattern_span,
                    })
                    .collect(),
            }),
            *span,
        ),

        Expr::Range {
            start,
            end,
            kind,
            span,
        } => Expr::Range {
            start: start
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            end: end
                .as_ref()
                .map(|e| Box::new(substitute_const_in_expr(e, const_subs))),
            kind: *kind,
            span: *span,
        },

        Expr::TimeframeContext {
            timeframe,
            expr,
            span,
        } => Expr::TimeframeContext {
            timeframe: timeframe.clone(),
            expr: Box::new(substitute_const_in_expr(expr, const_subs)),
            span: *span,
        },

        Expr::TryOperator(inner, span) => {
            Expr::TryOperator(Box::new(substitute_const_in_expr(inner, const_subs)), *span)
        }

        Expr::UsingImpl {
            expr,
            impl_name,
            span,
        } => Expr::UsingImpl {
            expr: Box::new(substitute_const_in_expr(expr, const_subs)),
            impl_name: impl_name.clone(),
            span: *span,
        },

        Expr::SimulationCall { name, params, span } => Expr::SimulationCall {
            name: name.clone(),
            params: params
                .iter()
                .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                .collect(),
            span: *span,
        },

        // WindowExpr is dense; its internal exprs are not part of the
        // const-generic call surface. Match substitute_expr's treatment.
        Expr::WindowExpr(w, span) => Expr::WindowExpr(w.clone(), *span),

        Expr::FromQuery(q, span) => Expr::FromQuery(
            Box::new(FromQueryExpr {
                variable: q.variable.clone(),
                source: Box::new(substitute_const_in_expr(&q.source, const_subs)),
                clauses: q
                    .clauses
                    .iter()
                    .map(|clause| match clause {
                        QueryClause::Where(e) => {
                            QueryClause::Where(Box::new(substitute_const_in_expr(e, const_subs)))
                        }
                        QueryClause::OrderBy(specs) => QueryClause::OrderBy(specs.clone()),
                        QueryClause::GroupBy {
                            element,
                            key,
                            into_var,
                        } => QueryClause::GroupBy {
                            element: Box::new(substitute_const_in_expr(element, const_subs)),
                            key: Box::new(substitute_const_in_expr(key, const_subs)),
                            into_var: into_var.clone(),
                        },
                        QueryClause::Join {
                            variable,
                            source,
                            left_key,
                            right_key,
                            into_var,
                        } => QueryClause::Join {
                            variable: variable.clone(),
                            source: Box::new(substitute_const_in_expr(source, const_subs)),
                            left_key: Box::new(substitute_const_in_expr(left_key, const_subs)),
                            right_key: Box::new(substitute_const_in_expr(right_key, const_subs)),
                            into_var: into_var.clone(),
                        },
                        QueryClause::Let { variable, value } => QueryClause::Let {
                            variable: variable.clone(),
                            value: Box::new(substitute_const_in_expr(value, const_subs)),
                        },
                    })
                    .collect(),
                select: Box::new(substitute_const_in_expr(&q.select, const_subs)),
            }),
            *span,
        ),

        Expr::StructLiteral {
            type_name,
            fields,
            span,
        } => Expr::StructLiteral {
            type_name: type_name.clone(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.clone(), substitute_const_in_expr(v, const_subs)))
                .collect(),
            span: *span,
        },

        Expr::Await(inner, span) => {
            Expr::Await(Box::new(substitute_const_in_expr(inner, const_subs)), *span)
        }

        Expr::Join(join, span) => Expr::Join(
            Box::new(JoinExpr {
                kind: join.kind,
                branches: join
                    .branches
                    .iter()
                    .map(|b| JoinBranch {
                        label: b.label.clone(),
                        expr: substitute_const_in_expr(&b.expr, const_subs),
                        annotations: b.annotations.clone(),
                    })
                    .collect(),
                span: join.span,
            }),
            *span,
        ),

        Expr::Annotated {
            annotation,
            target,
            span,
        } => Expr::Annotated {
            annotation: annotation.clone(),
            target: Box::new(substitute_const_in_expr(target, const_subs)),
            span: *span,
        },

        Expr::AsyncLet(async_let, span) => Expr::AsyncLet(
            Box::new(AsyncLetExpr {
                name: async_let.name.clone(),
                expr: Box::new(substitute_const_in_expr(&async_let.expr, const_subs)),
                span: async_let.span,
            }),
            *span,
        ),

        Expr::AsyncScope(inner, span) => Expr::AsyncScope(
            Box::new(substitute_const_in_expr(inner, const_subs)),
            *span,
        ),

        Expr::Comptime(stmts, span) => Expr::Comptime(
            stmts
                .iter()
                .map(|s| substitute_const_in_statement(s, const_subs))
                .collect(),
            *span,
        ),

        Expr::ComptimeFor(comp_for, span) => Expr::ComptimeFor(
            Box::new(ComptimeForExpr {
                variable: comp_for.variable.clone(),
                iterable: Box::new(substitute_const_in_expr(&comp_for.iterable, const_subs)),
                body: comp_for
                    .body
                    .iter()
                    .map(|s| substitute_const_in_statement(s, const_subs))
                    .collect(),
            }),
            *span,
        ),

        Expr::Reference {
            expr,
            is_mutable,
            span,
        } => Expr::Reference {
            expr: Box::new(substitute_const_in_expr(expr, const_subs)),
            is_mutable: *is_mutable,
            span: *span,
        },
    }
}

/// Convert a `ComptimeConstValue` into an `Expr` literal node so it can be
/// substituted into an expression position.
///
/// Every variant of `ComptimeConstValue` maps to a literal, so this always
/// returns `Some`. The `Option` return type is preserved for API compatibility
/// with callers that pattern-match on it.
fn const_value_to_literal(value: &ComptimeConstValue, span: shape_ast::ast::Span) -> Option<Expr> {
    use shape_ast::ast::Literal;
    match value {
        ComptimeConstValue::Int(i) => Some(Expr::Literal(Literal::Int(*i), span)),
        ComptimeConstValue::Bool(b) => Some(Expr::Literal(Literal::Bool(*b), span)),
        ComptimeConstValue::Number(f) => Some(Expr::Literal(Literal::Number(*f), span)),
        ComptimeConstValue::String(s) => Some(Expr::Literal(Literal::String(s.clone()), span)),
    }
}

// ---------------------------------------------------------------------------
// Helpers — walk every AST node that can carry a TypeAnnotation
// ---------------------------------------------------------------------------

fn substitute_function_parameter(
    p: &FunctionParameter,
    subs: &HashMap<String, ConcreteType>,
) -> FunctionParameter {
    FunctionParameter {
        pattern: substitute_destructure_pattern(&p.pattern, subs),
        is_const: p.is_const,
        is_reference: p.is_reference,
        is_mut_reference: p.is_mut_reference,
        is_out: p.is_out,
        type_annotation: p
            .type_annotation
            .as_ref()
            .map(|t| substitute_type_annotation(t, subs)),
        default_value: p.default_value.as_ref().map(|e| substitute_expr(e, subs)),
    }
}

fn substitute_destructure_pattern(
    pat: &DestructurePattern,
    subs: &HashMap<String, ConcreteType>,
) -> DestructurePattern {
    match pat {
        DestructurePattern::Identifier(name, span) => {
            DestructurePattern::Identifier(name.clone(), *span)
        }
        DestructurePattern::Array(items) => DestructurePattern::Array(
            items
                .iter()
                .map(|p| substitute_destructure_pattern(p, subs))
                .collect(),
        ),
        DestructurePattern::Object(fields) => DestructurePattern::Object(
            fields
                .iter()
                .map(|f| ObjectPatternField {
                    key: f.key.clone(),
                    pattern: substitute_destructure_pattern(&f.pattern, subs),
                })
                .collect(),
        ),
        DestructurePattern::Rest(inner) => {
            DestructurePattern::Rest(Box::new(substitute_destructure_pattern(inner, subs)))
        }
        DestructurePattern::Decomposition(bindings) => DestructurePattern::Decomposition(
            bindings
                .iter()
                .map(|b| DecompositionBinding {
                    name: b.name.clone(),
                    type_annotation: substitute_type_annotation(&b.type_annotation, subs),
                    span: b.span,
                })
                .collect(),
        ),
    }
}

fn substitute_pattern(pat: &Pattern, subs: &HashMap<String, ConcreteType>) -> Pattern {
    match pat {
        Pattern::Typed {
            name,
            type_annotation,
        } => Pattern::Typed {
            name: name.clone(),
            type_annotation: substitute_type_annotation(type_annotation, subs),
        },
        Pattern::Array(items) => {
            Pattern::Array(items.iter().map(|p| substitute_pattern(p, subs)).collect())
        }
        Pattern::Object(fields) => Pattern::Object(
            fields
                .iter()
                .map(|(k, p)| (k.clone(), substitute_pattern(p, subs)))
                .collect(),
        ),
        Pattern::Constructor {
            enum_name,
            variant,
            fields,
        } => Pattern::Constructor {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            fields: match fields {
                PatternConstructorFields::Unit => PatternConstructorFields::Unit,
                PatternConstructorFields::Tuple(pats) => PatternConstructorFields::Tuple(
                    pats.iter().map(|p| substitute_pattern(p, subs)).collect(),
                ),
                PatternConstructorFields::Struct(pairs) => PatternConstructorFields::Struct(
                    pairs
                        .iter()
                        .map(|(k, p)| (k.clone(), substitute_pattern(p, subs)))
                        .collect(),
                ),
            },
        },
        // Patterns with no nested annotations: pass through.
        Pattern::Identifier(_) | Pattern::Literal(_) | Pattern::Wildcard => pat.clone(),
    }
}

fn substitute_statement(stmt: &Statement, subs: &HashMap<String, ConcreteType>) -> Statement {
    match stmt {
        Statement::Return(expr, span) => {
            Statement::Return(expr.as_ref().map(|e| substitute_expr(e, subs)), *span)
        }
        Statement::Break(span) => Statement::Break(*span),
        Statement::Continue(span) => Statement::Continue(*span),

        Statement::VariableDecl(decl, span) => {
            let mut new_decl = decl.clone();
            new_decl.pattern = substitute_destructure_pattern(&decl.pattern, subs);
            new_decl.type_annotation = decl
                .type_annotation
                .as_ref()
                .map(|t| substitute_type_annotation(t, subs));
            new_decl.value = decl.value.as_ref().map(|e| substitute_expr(e, subs));
            Statement::VariableDecl(new_decl, *span)
        }

        Statement::Assignment(assign, span) => {
            let mut new_assign = assign.clone();
            new_assign.pattern = substitute_destructure_pattern(&assign.pattern, subs);
            new_assign.value = substitute_expr(&assign.value, subs);
            Statement::Assignment(new_assign, *span)
        }

        Statement::Expression(expr, span) => {
            Statement::Expression(substitute_expr(expr, subs), *span)
        }

        Statement::For(for_loop, span) => {
            let mut new_loop = for_loop.clone();
            new_loop.init = match &for_loop.init {
                ForInit::ForIn { pattern, iter } => ForInit::ForIn {
                    pattern: substitute_destructure_pattern(pattern, subs),
                    iter: substitute_expr(iter, subs),
                },
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => ForInit::ForC {
                    init: Box::new(substitute_statement(init, subs)),
                    condition: substitute_expr(condition, subs),
                    update: substitute_expr(update, subs),
                },
            };
            new_loop.body = for_loop
                .body
                .iter()
                .map(|s| substitute_statement(s, subs))
                .collect();
            Statement::For(new_loop, *span)
        }

        Statement::While(while_loop, span) => Statement::While(
            WhileLoop {
                condition: substitute_expr(&while_loop.condition, subs),
                body: while_loop
                    .body
                    .iter()
                    .map(|s| substitute_statement(s, subs))
                    .collect(),
            },
            *span,
        ),

        Statement::If(if_stmt, span) => Statement::If(
            IfStatement {
                condition: substitute_expr(&if_stmt.condition, subs),
                then_body: if_stmt
                    .then_body
                    .iter()
                    .map(|s| substitute_statement(s, subs))
                    .collect(),
                else_body: if_stmt.else_body.as_ref().map(|body| {
                    body.iter()
                        .map(|s| substitute_statement(s, subs))
                        .collect()
                }),
            },
            *span,
        ),

        Statement::Extend(ext, span) => Statement::Extend(substitute_extend(ext, subs), *span),

        Statement::RemoveTarget(span) => Statement::RemoveTarget(*span),

        Statement::SetParamType {
            param_name,
            type_annotation,
            span,
        } => Statement::SetParamType {
            param_name: param_name.clone(),
            type_annotation: substitute_type_annotation(type_annotation, subs),
            span: *span,
        },

        Statement::SetParamValue {
            param_name,
            expression,
            span,
        } => Statement::SetParamValue {
            param_name: param_name.clone(),
            expression: substitute_expr(expression, subs),
            span: *span,
        },

        Statement::SetReturnType {
            type_annotation,
            span,
        } => Statement::SetReturnType {
            type_annotation: substitute_type_annotation(type_annotation, subs),
            span: *span,
        },

        Statement::SetReturnExpr { expression, span } => Statement::SetReturnExpr {
            expression: substitute_expr(expression, subs),
            span: *span,
        },

        Statement::ReplaceBody { body, span } => Statement::ReplaceBody {
            body: body
                .iter()
                .map(|s| substitute_statement(s, subs))
                .collect(),
            span: *span,
        },

        Statement::ReplaceBodyExpr { expression, span } => Statement::ReplaceBodyExpr {
            expression: substitute_expr(expression, subs),
            span: *span,
        },

        Statement::ReplaceModuleExpr { expression, span } => Statement::ReplaceModuleExpr {
            expression: substitute_expr(expression, subs),
            span: *span,
        },
    }
}

fn substitute_extend(
    ext: &ExtendStatement,
    subs: &HashMap<String, ConcreteType>,
) -> ExtendStatement {
    ExtendStatement {
        type_name: ext.type_name.clone(),
        methods: ext
            .methods
            .iter()
            .map(|m| substitute_method_def(m, subs))
            .collect(),
    }
}

fn substitute_method_def(m: &MethodDef, subs: &HashMap<String, ConcreteType>) -> MethodDef {
    let mut cloned = m.clone();
    cloned.params = m
        .params
        .iter()
        .map(|p| substitute_function_parameter(p, subs))
        .collect();
    cloned.return_type = m
        .return_type
        .as_ref()
        .map(|t| substitute_type_annotation(t, subs));
    cloned.body = m
        .body
        .iter()
        .map(|s| substitute_statement(s, subs))
        .collect();
    cloned.when_clause = m
        .when_clause
        .as_ref()
        .map(|e| Box::new(substitute_expr(e, subs)));
    cloned
}

fn substitute_expr(expr: &Expr, subs: &HashMap<String, ConcreteType>) -> Expr {
    match expr {
        // Leaves: nothing to recurse into.
        Expr::Literal(_, _)
        | Expr::Identifier(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::Duration(_, _)
        | Expr::Continue(_)
        | Expr::Unit(_)
        | Expr::TableRows(_, _) => expr.clone(),

        Expr::DataRelativeAccess {
            reference,
            index,
            span,
        } => Expr::DataRelativeAccess {
            reference: Box::new(substitute_expr(reference, subs)),
            index: index.clone(),
            span: *span,
        },

        Expr::PropertyAccess {
            object,
            property,
            optional,
            span,
        } => Expr::PropertyAccess {
            object: Box::new(substitute_expr(object, subs)),
            property: property.clone(),
            optional: *optional,
            span: *span,
        },

        Expr::IndexAccess {
            object,
            index,
            end_index,
            span,
        } => Expr::IndexAccess {
            object: Box::new(substitute_expr(object, subs)),
            index: Box::new(substitute_expr(index, subs)),
            end_index: end_index
                .as_ref()
                .map(|e| Box::new(substitute_expr(e, subs))),
            span: *span,
        },

        Expr::BinaryOp {
            left,
            op,
            right,
            span,
        } => Expr::BinaryOp {
            left: Box::new(substitute_expr(left, subs)),
            op: op.clone(),
            right: Box::new(substitute_expr(right, subs)),
            span: *span,
        },

        Expr::FuzzyComparison {
            left,
            op,
            right,
            tolerance,
            span,
        } => Expr::FuzzyComparison {
            left: Box::new(substitute_expr(left, subs)),
            op: op.clone(),
            right: Box::new(substitute_expr(right, subs)),
            tolerance: tolerance.clone(),
            span: *span,
        },

        Expr::UnaryOp { op, operand, span } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(substitute_expr(operand, subs)),
            span: *span,
        },

        Expr::FunctionCall {
            name,
            args,
            named_args,
            span,
        } => Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_expr(a, subs)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                .collect(),
            span: *span,
        },

        Expr::QualifiedFunctionCall {
            namespace,
            function,
            args,
            named_args,
            span,
        } => Expr::QualifiedFunctionCall {
            namespace: namespace.clone(),
            function: function.clone(),
            args: args.iter().map(|a| substitute_expr(a, subs)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                .collect(),
            span: *span,
        },

        Expr::EnumConstructor {
            enum_name,
            variant,
            payload,
            span,
        } => Expr::EnumConstructor {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: match payload {
                EnumConstructorPayload::Unit => EnumConstructorPayload::Unit,
                EnumConstructorPayload::Tuple(args) => EnumConstructorPayload::Tuple(
                    args.iter().map(|a| substitute_expr(a, subs)).collect(),
                ),
                EnumConstructorPayload::Struct(fields) => EnumConstructorPayload::Struct(
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                        .collect(),
                ),
            },
            span: *span,
        },

        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            span,
        } => Expr::Conditional {
            condition: Box::new(substitute_expr(condition, subs)),
            then_expr: Box::new(substitute_expr(then_expr, subs)),
            else_expr: else_expr
                .as_ref()
                .map(|e| Box::new(substitute_expr(e, subs))),
            span: *span,
        },

        Expr::Object(entries, span) => Expr::Object(
            entries
                .iter()
                .map(|e| match e {
                    ObjectEntry::Field {
                        key,
                        value,
                        type_annotation,
                    } => ObjectEntry::Field {
                        key: key.clone(),
                        value: substitute_expr(value, subs),
                        type_annotation: type_annotation
                            .as_ref()
                            .map(|t| substitute_type_annotation(t, subs)),
                    },
                    ObjectEntry::Spread(inner) => ObjectEntry::Spread(substitute_expr(inner, subs)),
                })
                .collect(),
            *span,
        ),

        Expr::Array(items, span) => Expr::Array(
            items.iter().map(|i| substitute_expr(i, subs)).collect(),
            *span,
        ),

        Expr::ListComprehension(comp, span) => Expr::ListComprehension(
            Box::new(ListComprehension {
                element: Box::new(substitute_expr(&comp.element, subs)),
                clauses: comp
                    .clauses
                    .iter()
                    .map(|c| ComprehensionClause {
                        pattern: substitute_destructure_pattern(&c.pattern, subs),
                        iterable: Box::new(substitute_expr(&c.iterable, subs)),
                        filter: c
                            .filter
                            .as_ref()
                            .map(|f| Box::new(substitute_expr(f, subs))),
                    })
                    .collect(),
            }),
            *span,
        ),

        Expr::Block(block, span) => {
            let new_items = block
                .items
                .iter()
                .map(|item| match item {
                    BlockItem::VariableDecl(decl) => {
                        let mut new_decl = decl.clone();
                        new_decl.pattern = substitute_destructure_pattern(&decl.pattern, subs);
                        new_decl.type_annotation = decl
                            .type_annotation
                            .as_ref()
                            .map(|t| substitute_type_annotation(t, subs));
                        new_decl.value = decl.value.as_ref().map(|e| substitute_expr(e, subs));
                        BlockItem::VariableDecl(new_decl)
                    }
                    BlockItem::Assignment(assign) => {
                        let mut new_assign = assign.clone();
                        new_assign.pattern =
                            substitute_destructure_pattern(&assign.pattern, subs);
                        new_assign.value = substitute_expr(&assign.value, subs);
                        BlockItem::Assignment(new_assign)
                    }
                    BlockItem::Statement(s) => BlockItem::Statement(substitute_statement(s, subs)),
                    BlockItem::Expression(e) => BlockItem::Expression(substitute_expr(e, subs)),
                })
                .collect();
            Expr::Block(BlockExpr { items: new_items }, *span)
        }

        Expr::TypeAssertion {
            expr,
            type_annotation,
            meta_param_overrides,
            span,
        } => Expr::TypeAssertion {
            expr: Box::new(substitute_expr(expr, subs)),
            type_annotation: substitute_type_annotation(type_annotation, subs),
            meta_param_overrides: meta_param_overrides.clone(),
            span: *span,
        },

        Expr::InstanceOf {
            expr,
            type_annotation,
            span,
        } => Expr::InstanceOf {
            expr: Box::new(substitute_expr(expr, subs)),
            type_annotation: substitute_type_annotation(type_annotation, subs),
            span: *span,
        },

        Expr::FunctionExpr {
            params,
            return_type,
            body,
            span,
        } => Expr::FunctionExpr {
            params: params
                .iter()
                .map(|p| substitute_function_parameter(p, subs))
                .collect(),
            return_type: return_type
                .as_ref()
                .map(|t| substitute_type_annotation(t, subs)),
            body: body.iter().map(|s| substitute_statement(s, subs)).collect(),
            span: *span,
        },

        Expr::Spread(inner, span) => {
            Expr::Spread(Box::new(substitute_expr(inner, subs)), *span)
        }

        Expr::If(if_expr, span) => Expr::If(
            Box::new(IfExpr {
                condition: Box::new(substitute_expr(&if_expr.condition, subs)),
                then_branch: Box::new(substitute_expr(&if_expr.then_branch, subs)),
                else_branch: if_expr
                    .else_branch
                    .as_ref()
                    .map(|e| Box::new(substitute_expr(e, subs))),
            }),
            *span,
        ),

        Expr::While(while_expr, span) => Expr::While(
            Box::new(WhileExpr {
                condition: Box::new(substitute_expr(&while_expr.condition, subs)),
                body: Box::new(substitute_expr(&while_expr.body, subs)),
            }),
            *span,
        ),

        Expr::For(for_expr, span) => Expr::For(
            Box::new(ForExpr {
                pattern: substitute_pattern(&for_expr.pattern, subs),
                iterable: Box::new(substitute_expr(&for_expr.iterable, subs)),
                body: Box::new(substitute_expr(&for_expr.body, subs)),
                is_async: for_expr.is_async,
            }),
            *span,
        ),

        Expr::Loop(loop_expr, span) => Expr::Loop(
            Box::new(LoopExpr {
                body: Box::new(substitute_expr(&loop_expr.body, subs)),
            }),
            *span,
        ),

        Expr::Let(let_expr, span) => Expr::Let(
            Box::new(LetExpr {
                pattern: substitute_pattern(&let_expr.pattern, subs),
                type_annotation: let_expr
                    .type_annotation
                    .as_ref()
                    .map(|t| substitute_type_annotation(t, subs)),
                value: let_expr
                    .value
                    .as_ref()
                    .map(|v| Box::new(substitute_expr(v, subs))),
                body: Box::new(substitute_expr(&let_expr.body, subs)),
            }),
            *span,
        ),

        Expr::Assign(assign_expr, span) => Expr::Assign(
            Box::new(AssignExpr {
                target: Box::new(substitute_expr(&assign_expr.target, subs)),
                value: Box::new(substitute_expr(&assign_expr.value, subs)),
            }),
            *span,
        ),

        Expr::Break(value, span) => Expr::Break(
            value.as_ref().map(|e| Box::new(substitute_expr(e, subs))),
            *span,
        ),

        Expr::Return(value, span) => Expr::Return(
            value.as_ref().map(|e| Box::new(substitute_expr(e, subs))),
            *span,
        ),

        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            optional,
            span,
        } => Expr::MethodCall {
            receiver: Box::new(substitute_expr(receiver, subs)),
            method: method.clone(),
            args: args.iter().map(|a| substitute_expr(a, subs)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                .collect(),
            optional: *optional,
            span: *span,
        },

        Expr::Match(match_expr, span) => Expr::Match(
            Box::new(MatchExpr {
                scrutinee: Box::new(substitute_expr(&match_expr.scrutinee, subs)),
                arms: match_expr
                    .arms
                    .iter()
                    .map(|arm| MatchArm {
                        pattern: substitute_pattern(&arm.pattern, subs),
                        guard: arm.guard.as_ref().map(|g| Box::new(substitute_expr(g, subs))),
                        body: Box::new(substitute_expr(&arm.body, subs)),
                        pattern_span: arm.pattern_span,
                    })
                    .collect(),
            }),
            *span,
        ),

        Expr::Range {
            start,
            end,
            kind,
            span,
        } => Expr::Range {
            start: start.as_ref().map(|e| Box::new(substitute_expr(e, subs))),
            end: end.as_ref().map(|e| Box::new(substitute_expr(e, subs))),
            kind: *kind,
            span: *span,
        },

        Expr::TimeframeContext {
            timeframe,
            expr,
            span,
        } => Expr::TimeframeContext {
            timeframe: timeframe.clone(),
            expr: Box::new(substitute_expr(expr, subs)),
            span: *span,
        },

        Expr::TryOperator(inner, span) => {
            Expr::TryOperator(Box::new(substitute_expr(inner, subs)), *span)
        }

        Expr::UsingImpl {
            expr,
            impl_name,
            span,
        } => Expr::UsingImpl {
            expr: Box::new(substitute_expr(expr, subs)),
            impl_name: impl_name.clone(),
            span: *span,
        },

        Expr::SimulationCall { name, params, span } => Expr::SimulationCall {
            name: name.clone(),
            params: params
                .iter()
                .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                .collect(),
            span: *span,
        },

        // WindowExpr is dense and very rarely appears inside generic
        // function bodies the monomorphizer handles. Structural clone is
        // safe — TypeAnnotations inside WindowExpr aren't part of the
        // generic-function call paths exercised by Phase 2.1.
        Expr::WindowExpr(w, span) => Expr::WindowExpr(w.clone(), *span),

        Expr::FromQuery(q, span) => Expr::FromQuery(
            Box::new(FromQueryExpr {
                variable: q.variable.clone(),
                source: Box::new(substitute_expr(&q.source, subs)),
                clauses: q
                    .clauses
                    .iter()
                    .map(|clause| match clause {
                        QueryClause::Where(e) => {
                            QueryClause::Where(Box::new(substitute_expr(e, subs)))
                        }
                        QueryClause::OrderBy(specs) => QueryClause::OrderBy(specs.clone()),
                        QueryClause::GroupBy {
                            element,
                            key,
                            into_var,
                        } => QueryClause::GroupBy {
                            element: Box::new(substitute_expr(element, subs)),
                            key: Box::new(substitute_expr(key, subs)),
                            into_var: into_var.clone(),
                        },
                        QueryClause::Join {
                            variable,
                            source,
                            left_key,
                            right_key,
                            into_var,
                        } => QueryClause::Join {
                            variable: variable.clone(),
                            source: Box::new(substitute_expr(source, subs)),
                            left_key: Box::new(substitute_expr(left_key, subs)),
                            right_key: Box::new(substitute_expr(right_key, subs)),
                            into_var: into_var.clone(),
                        },
                        QueryClause::Let { variable, value } => QueryClause::Let {
                            variable: variable.clone(),
                            value: Box::new(substitute_expr(value, subs)),
                        },
                    })
                    .collect(),
                select: Box::new(substitute_expr(&q.select, subs)),
            }),
            *span,
        ),

        Expr::StructLiteral {
            type_name,
            fields,
            span,
        } => Expr::StructLiteral {
            type_name: type_name.clone(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.clone(), substitute_expr(v, subs)))
                .collect(),
            span: *span,
        },

        Expr::Await(inner, span) => Expr::Await(Box::new(substitute_expr(inner, subs)), *span),

        Expr::Join(join, span) => Expr::Join(
            Box::new(JoinExpr {
                kind: join.kind,
                branches: join
                    .branches
                    .iter()
                    .map(|b| JoinBranch {
                        label: b.label.clone(),
                        expr: substitute_expr(&b.expr, subs),
                        annotations: b.annotations.clone(),
                    })
                    .collect(),
                span: join.span,
            }),
            *span,
        ),

        Expr::Annotated {
            annotation,
            target,
            span,
        } => Expr::Annotated {
            annotation: annotation.clone(),
            target: Box::new(substitute_expr(target, subs)),
            span: *span,
        },

        Expr::AsyncLet(async_let, span) => Expr::AsyncLet(
            Box::new(AsyncLetExpr {
                name: async_let.name.clone(),
                expr: Box::new(substitute_expr(&async_let.expr, subs)),
                span: async_let.span,
            }),
            *span,
        ),

        Expr::AsyncScope(inner, span) => {
            Expr::AsyncScope(Box::new(substitute_expr(inner, subs)), *span)
        }

        Expr::Comptime(stmts, span) => Expr::Comptime(
            stmts
                .iter()
                .map(|s| substitute_statement(s, subs))
                .collect(),
            *span,
        ),

        Expr::ComptimeFor(comp_for, span) => Expr::ComptimeFor(
            Box::new(ComptimeForExpr {
                variable: comp_for.variable.clone(),
                iterable: Box::new(substitute_expr(&comp_for.iterable, subs)),
                body: comp_for
                    .body
                    .iter()
                    .map(|s| substitute_statement(s, subs))
                    .collect(),
            }),
            *span,
        ),

        Expr::Reference {
            expr,
            is_mutable,
            span,
        } => Expr::Reference {
            expr: Box::new(substitute_expr(expr, subs)),
            is_mutable: *is_mutable,
            span: *span,
        },
    }
}

// ---------------------------------------------------------------------------
// Phase C — closure body inlining
// ---------------------------------------------------------------------------

/// Phase C — inline a closure literal's body into a specialized stdlib
/// template.
///
/// Given a specialized function body (e.g. the result of
/// `substitute_function_def` on `Vec.map`) that names a formal closure
/// parameter (e.g. `f: (T) => U`), and given the caller's closure literal
/// (`|x| x + n`), this helper rewrites every `Expr::FunctionCall { name: "f", args }`
/// inside the specialized body into the closure's body with its formal
/// parameters substituted by the call's argument expressions.
///
/// Captures become **leading parameters of the specialized body**: the helper
/// prepends one `FunctionParameter` per capture (drawn from `capture_names`)
/// to `specialized.params`. The specialized body sees captures as plain local
/// identifiers, so lexical references inside the inlined closure body
/// (`n` in the example above) resolve like any other local parameter.
///
/// # Arguments
///
/// * `specialized` — The specialized function def. Mutated in place: its
///   `params` gain capture-prefix entries and its `body` is rewritten.
/// * `closure_param_name` — The name of the formal closure parameter inside
///   `specialized.params` that should be replaced. After inlining, this
///   parameter is REMOVED from `specialized.params`. Callers typically pass
///   the name from the original (pre-substitution) def.
/// * `closure_params` — The closure literal's formal param names in
///   positional order. Used to substitute call arguments into the closure
///   body.
/// * `closure_body` — The closure literal's body statements.
/// * `capture_names` — Names of the closure's captures in the order they
///   appear in the capture list. Each one becomes a leading parameter of the
///   specialized body (untyped — the bytecode compiler infers from usage).
///
/// # Return
///
/// `Ok(())` when inlining succeeds. `Err(_)` when the closure body has a
/// shape the inliner doesn't yet handle — callers should fall through to the
/// non-inlining specialization path in that case.
///
/// # Worked example (map with 1-capture closure)
///
/// Specialized template (before):
/// ```shape
/// fn map::i64(self: Array<int>, f: (int) => int) -> Array<int> {
///     let mut result = []
///     for item in self {
///         result.push(f(item))
///     }
///     result
/// }
/// ```
///
/// Closure literal `|x| x + n` with `n` captured:
///
/// After `inline_closure_body_into_specialization(&mut spec, "f", &["x"], body_of_closure, &["n"])`:
/// ```shape
/// fn map::i64_closure_7_i64(n: _, self: Array<int>) -> Array<int> {
///     let mut result = []
///     for item in self {
///         result.push({ item + n })   // inlined: `x` → `item`
///     }
///     result
/// }
/// ```
pub fn inline_closure_body_into_specialization(
    specialized: &mut shape_ast::ast::FunctionDef,
    closure_param_name: &str,
    closure_params: &[String],
    closure_body: &[shape_ast::ast::Statement],
    capture_names: &[String],
) -> shape_ast::error::Result<()> {
    // Walk the body and replace every call to the formal closure parameter
    // (`f(item)`) with the inlined closure body.
    //
    // Phase C scope note — we intentionally DO NOT strip `closure_param_name`
    // from the specialized function's parameter list, and DO NOT hoist
    // captures into leading parameters. The call site still emits
    // `MakeClosure` + the closure-arg slot, and the specialized body simply
    // never invokes `f` — the closure pointer sits unused in its local slot.
    //
    // This keeps the call-site ABI identical to the pre-Phase-C direct-call
    // path (`Call(specialized_idx)` with the original arg list), which means:
    //
    //   1. No new call-site compiler changes are required to land Phase C.
    //   2. The win is still real: the specialized body contains ZERO
    //      `CallValue`/`CallClosure` opcodes — the closure body is inlined.
    //   3. Zero-alloc + capture hoisting are Phase D/E/H work (see the
    //      closure-specialization design doc §4 and §5); they strip the
    //      formal closure param + replace `MakeClosure` with stack
    //      construction. Until those phases land, keeping the formal param
    //      preserves ABI compatibility with unspecialized call sites.
    //
    // The `capture_names` arg is accepted for API stability but unused here
    // — it becomes live once Phase D/E wires up capture hoisting.
    let _ = capture_names;

    // cluster-2-cw-2-phaseC-inlining empirical-verification trace
    // (2026-05-16). Per cluster-2-v3s6f-empirical-verification.md §3.4,
    // disposition between the 4 explanations (AST-vs-MIR retention,
    // name mismatch, recursion descent gap, post-Phase-C re-introduction)
    // requires SHAPE_JIT_DEBUG-gated visibility into the AST visit
    // pattern inside `inline_closure_calls_in_expr`. The thread-local
    // counter below is the smallest empirical instrument: it counts
    // every FunctionCall visit AND its name vs `closure_param_name`
    // comparison outcome. Surface name MISMATCHES (explanation 2) +
    // MethodCall/MethodCall.args recursion (explanation 3) + AST shape
    // dump of the specialized body before and after Phase-C (explanation
    // 1) are recorded simultaneously per single empirical pass.
    if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
        PHASEC_TRACE_FN_CALL_TOTAL.with(|c| c.set(0));
        PHASEC_TRACE_FN_CALL_MATCH.with(|c| c.set(0));
        PHASEC_TRACE_METHOD_CALL.with(|c| c.set(0));
        PHASEC_TRACE_FOR_STMT.with(|c| c.set(0));
        eprintln!(
            "[phaseC-empirical] specialization fn={} body stmt count BEFORE inline = {}",
            specialized.name,
            specialized.body.len(),
        );
        // Dump every top-level statement discriminant so we can confirm
        // the for-loop carrying f(item) is present.
        for (i, s) in specialized.body.iter().enumerate() {
            eprintln!(
                "[phaseC-empirical]   pre-body[{}] discriminant={}",
                i,
                statement_discriminant(s),
            );
        }
    }

    specialized.body = specialized
        .body
        .iter()
        .map(|s| {
            inline_closure_calls_in_statement(
                s,
                closure_param_name,
                closure_params,
                closure_body,
            )
        })
        .collect();

    if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
        eprintln!(
            "[phaseC-empirical] specialization fn={} body stmt count AFTER inline = {} \
             fn_call_total={} fn_call_match={} method_call={} for_stmt={}",
            specialized.name,
            specialized.body.len(),
            PHASEC_TRACE_FN_CALL_TOTAL.with(|c| c.get()),
            PHASEC_TRACE_FN_CALL_MATCH.with(|c| c.get()),
            PHASEC_TRACE_METHOD_CALL.with(|c| c.get()),
            PHASEC_TRACE_FOR_STMT.with(|c| c.get()),
        );
        for (i, s) in specialized.body.iter().enumerate() {
            eprintln!(
                "[phaseC-empirical]   post-body[{}] discriminant={}",
                i,
                statement_discriminant(s),
            );
        }
    }

    Ok(())
}

// cluster-2-cw-2-phaseC-inlining empirical trace counters (2026-05-16).
// SHAPE_JIT_DEBUG-gated; reset per Phase-C invocation. See
// inline_closure_body_into_specialization for the instrumentation
// rationale (§3.4 of cluster-2-v3s6f-empirical-verification.md, 4
// explanations dispositionable per single empirical pass).
thread_local! {
    static PHASEC_TRACE_FN_CALL_TOTAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PHASEC_TRACE_FN_CALL_MATCH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PHASEC_TRACE_METHOD_CALL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PHASEC_TRACE_FOR_STMT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn statement_discriminant(s: &shape_ast::ast::Statement) -> &'static str {
    use shape_ast::ast::Statement;
    match s {
        Statement::Return(..) => "Return",
        Statement::Expression(..) => "Expression",
        Statement::VariableDecl(..) => "VariableDecl",
        Statement::Assignment(..) => "Assignment",
        Statement::For(..) => "For",
        Statement::While(..) => "While",
        Statement::If(..) => "If",
        Statement::Break(..) => "Break",
        Statement::Continue(..) => "Continue",
        Statement::Extend(..) => "Extend",
        _ => "Other",
    }
}

/// Recursively walk a statement and rewrite any `Expr::FunctionCall` whose
/// name is `closure_param_name` into a block that (a) binds the closure's
/// formal params to the call's actual args and (b) executes the closure body.
fn inline_closure_calls_in_statement(
    stmt: &shape_ast::ast::Statement,
    closure_param_name: &str,
    closure_params: &[String],
    closure_body: &[shape_ast::ast::Statement],
) -> shape_ast::ast::Statement {
    use shape_ast::ast::{Statement, statements::{ForInit, ForLoop, IfStatement, WhileLoop}};
    match stmt {
        Statement::Return(expr, span) => Statement::Return(
            expr.as_ref()
                .map(|e| inline_closure_calls_in_expr(e, closure_param_name, closure_params, closure_body)),
            *span,
        ),
        Statement::Expression(expr, span) => Statement::Expression(
            inline_closure_calls_in_expr(expr, closure_param_name, closure_params, closure_body),
            *span,
        ),
        Statement::VariableDecl(decl, span) => {
            let mut new_decl = decl.clone();
            new_decl.value = decl.value.as_ref().map(|e| {
                inline_closure_calls_in_expr(e, closure_param_name, closure_params, closure_body)
            });
            Statement::VariableDecl(new_decl, *span)
        }
        Statement::Assignment(assignment, span) => {
            let mut new_assign = assignment.clone();
            new_assign.value = inline_closure_calls_in_expr(
                &assignment.value,
                closure_param_name,
                closure_params,
                closure_body,
            );
            Statement::Assignment(new_assign, *span)
        }
        Statement::For(for_loop, span) => {
            // cluster-2-cw-2-phaseC-inlining empirical trace (2026-05-16).
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                PHASEC_TRACE_FOR_STMT.with(|c| c.set(c.get() + 1));
                eprintln!(
                    "[phaseC-empirical]   For statement encountered (body_stmts={})",
                    for_loop.body.len(),
                );
            }
            let new_init = match &for_loop.init {
                ForInit::ForIn { pattern, iter } => ForInit::ForIn {
                    pattern: pattern.clone(),
                    iter: inline_closure_calls_in_expr(
                        iter,
                        closure_param_name,
                        closure_params,
                        closure_body,
                    ),
                },
                ForInit::ForC { init, condition, update } => ForInit::ForC {
                    init: Box::new(inline_closure_calls_in_statement(
                        init,
                        closure_param_name,
                        closure_params,
                        closure_body,
                    )),
                    condition: inline_closure_calls_in_expr(
                        condition,
                        closure_param_name,
                        closure_params,
                        closure_body,
                    ),
                    update: inline_closure_calls_in_expr(
                        update,
                        closure_param_name,
                        closure_params,
                        closure_body,
                    ),
                },
            };
            Statement::For(
                ForLoop {
                    init: new_init,
                    body: for_loop
                        .body
                        .iter()
                        .map(|s| {
                            inline_closure_calls_in_statement(
                                s,
                                closure_param_name,
                                closure_params,
                                closure_body,
                            )
                        })
                        .collect(),
                    is_async: for_loop.is_async,
                },
                *span,
            )
        }
        Statement::While(wl, span) => Statement::While(
            WhileLoop {
                condition: inline_closure_calls_in_expr(
                    &wl.condition,
                    closure_param_name,
                    closure_params,
                    closure_body,
                ),
                body: wl
                    .body
                    .iter()
                    .map(|s| {
                        inline_closure_calls_in_statement(
                            s,
                            closure_param_name,
                            closure_params,
                            closure_body,
                        )
                    })
                    .collect(),
            },
            *span,
        ),
        Statement::If(ifs, span) => Statement::If(
            IfStatement {
                condition: inline_closure_calls_in_expr(
                    &ifs.condition,
                    closure_param_name,
                    closure_params,
                    closure_body,
                ),
                then_body: ifs
                    .then_body
                    .iter()
                    .map(|s| {
                        inline_closure_calls_in_statement(
                            s,
                            closure_param_name,
                            closure_params,
                            closure_body,
                        )
                    })
                    .collect(),
                else_body: ifs.else_body.as_ref().map(|body| {
                    body.iter()
                        .map(|s| {
                            inline_closure_calls_in_statement(
                                s,
                                closure_param_name,
                                closure_params,
                                closure_body,
                            )
                        })
                        .collect()
                }),
            },
            *span,
        ),
        // Pass-through: Break, Continue, comptime-only directives, Extend.
        // These don't occur in the stdlib map/filter/reduce bodies that
        // Phase C targets.
        other => other.clone(),
    }
}

/// Rewrite a `Expr::FunctionCall { name: closure_param_name, args }` into a
/// block expression that inlines the closure body. All other expression
/// shapes are walked structurally so the replacement reaches nested calls.
fn inline_closure_calls_in_expr(
    expr: &shape_ast::ast::Expr,
    closure_param_name: &str,
    closure_params: &[String],
    closure_body: &[shape_ast::ast::Statement],
) -> shape_ast::ast::Expr {
    use shape_ast::ast::Expr;
    // Intercept the target case first: a call whose name is the formal
    // closure parameter. We still want the args themselves to be processed
    // (they could contain nested calls), but swapping with the inlined body
    // happens here.
    if let Expr::FunctionCall { name, args, .. } = expr {
        // cluster-2-cw-2-phaseC-inlining empirical trace (2026-05-16).
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            PHASEC_TRACE_FN_CALL_TOTAL.with(|c| c.set(c.get() + 1));
            let matched = name == closure_param_name;
            if matched {
                PHASEC_TRACE_FN_CALL_MATCH.with(|c| c.set(c.get() + 1));
            }
            eprintln!(
                "[phaseC-empirical]   FunctionCall name={:?} closure_param={:?} matched={}",
                name, closure_param_name, matched,
            );
        }
        if name == closure_param_name {
            // Recursively walk each arg first so any nested closure-param
            // calls get replaced too.
            let rewritten_args: Vec<Expr> = args
                .iter()
                .map(|a| {
                    inline_closure_calls_in_expr(
                        a,
                        closure_param_name,
                        closure_params,
                        closure_body,
                    )
                })
                .collect();
            return build_inlined_closure_block(closure_params, &rewritten_args, closure_body);
        }
    }

    // Otherwise recurse structurally — a small top-down rewrite of every
    // Expr variant that can hold sub-expressions. This is verbose but
    // correct: any variant not matched here falls through unchanged.
    let rec = |e: &Expr| {
        inline_closure_calls_in_expr(e, closure_param_name, closure_params, closure_body)
    };
    let rec_box = |e: &Box<Expr>| Box::new(rec(e));
    let rec_vec = |v: &Vec<Expr>| v.iter().map(rec).collect();

    match expr {
        Expr::BinaryOp { left, op, right, span } => Expr::BinaryOp {
            left: rec_box(left),
            op: op.clone(),
            right: rec_box(right),
            span: *span,
        },
        Expr::UnaryOp { op, operand, span } => Expr::UnaryOp {
            op: op.clone(),
            operand: rec_box(operand),
            span: *span,
        },
        Expr::FunctionCall { name, args, named_args, span } => Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(rec).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), rec(v)))
                .collect(),
            span: *span,
        },
        Expr::MethodCall { receiver, method, args, named_args, optional, span } => {
            // cluster-2-cw-2-phaseC-inlining empirical trace (2026-05-16).
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                PHASEC_TRACE_METHOD_CALL.with(|c| c.set(c.get() + 1));
                eprintln!(
                    "[phaseC-empirical]   MethodCall method={:?} args_count={}",
                    method, args.len(),
                );
            }
            Expr::MethodCall {
                receiver: rec_box(receiver),
                method: method.clone(),
                args: rec_vec(args),
                named_args: named_args
                    .iter()
                    .map(|(k, v)| (k.clone(), rec(v)))
                    .collect(),
                optional: *optional,
                span: *span,
            }
        }
        Expr::PropertyAccess { object, property, optional, span } => Expr::PropertyAccess {
            object: rec_box(object),
            property: property.clone(),
            optional: *optional,
            span: *span,
        },
        Expr::IndexAccess { object, index, end_index, span } => Expr::IndexAccess {
            object: rec_box(object),
            index: rec_box(index),
            end_index: end_index.as_ref().map(|e| rec_box(e)),
            span: *span,
        },
        Expr::Array(items, span) => Expr::Array(rec_vec(items), *span),
        Expr::Assign(assign, span) => {
            use shape_ast::ast::expr_helpers::AssignExpr;
            Expr::Assign(
                Box::new(AssignExpr {
                    target: rec_box(&assign.target),
                    value: rec_box(&assign.value),
                }),
                *span,
            )
        }
        Expr::Return(Some(e), span) => Expr::Return(Some(rec_box(e)), *span),
        Expr::If(ifexpr, span) => {
            use shape_ast::ast::expr_helpers::IfExpr;
            Expr::If(
                Box::new(IfExpr {
                    condition: rec_box(&ifexpr.condition),
                    then_branch: rec_box(&ifexpr.then_branch),
                    else_branch: ifexpr.else_branch.as_ref().map(|e| rec_box(e)),
                }),
                *span,
            )
        }
        Expr::Block(block, span) => {
            use shape_ast::ast::expr_helpers::{BlockExpr, BlockItem};
            let items = block
                .items
                .iter()
                .map(|item| match item {
                    BlockItem::Expression(e) => BlockItem::Expression(rec(e)),
                    BlockItem::Statement(s) => BlockItem::Statement(
                        inline_closure_calls_in_statement(
                            s,
                            closure_param_name,
                            closure_params,
                            closure_body,
                        ),
                    ),
                    BlockItem::VariableDecl(decl) => {
                        let mut new_decl = decl.clone();
                        new_decl.value = decl.value.as_ref().map(rec);
                        BlockItem::VariableDecl(new_decl)
                    }
                    BlockItem::Assignment(assignment) => {
                        let mut na = assignment.clone();
                        na.value = rec(&assignment.value);
                        BlockItem::Assignment(na)
                    }
                })
                .collect();
            Expr::Block(BlockExpr { items }, *span)
        }
        // cluster-2-cw-2-phaseC-inlining (2026-05-16): missing arms that
        // wrap sub-expressions where a closure-parameter call may live.
        // Empirical verification at HEAD ca8300f0 confirmed (a)'s fix did
        // NOT make hypothesis (b) latent — the Vec.map body's
        // `for item in self { result.push(f(item)) }` was wrapped as
        // `Expr::For(...)` and silently passed through `other =>
        // other.clone()` below, so the inliner NEVER descended to the
        // `f(item)` call. Per cluster-2-v3s6f-empirical-verification.md
        // §3.4 explanation 3 (recursion-descent gap, now CONFIRMED at the
        // Expr layer rather than the MethodCall.args layer the original
        // §3.4 hypothesized): add explicit arms for every Expr variant
        // that can contain sub-expressions which can carry the formal
        // closure parameter call.
        Expr::For(for_expr, span) => {
            use shape_ast::ast::expr_helpers::ForExpr;
            Expr::For(
                Box::new(ForExpr {
                    pattern: for_expr.pattern.clone(),
                    iterable: rec_box(&for_expr.iterable),
                    body: rec_box(&for_expr.body),
                    is_async: for_expr.is_async,
                }),
                *span,
            )
        }
        Expr::While(while_expr, span) => {
            use shape_ast::ast::expr_helpers::WhileExpr;
            Expr::While(
                Box::new(WhileExpr {
                    condition: rec_box(&while_expr.condition),
                    body: rec_box(&while_expr.body),
                }),
                *span,
            )
        }
        Expr::Loop(loop_expr, span) => {
            use shape_ast::ast::expr_helpers::LoopExpr;
            Expr::Loop(
                Box::new(LoopExpr {
                    body: rec_box(&loop_expr.body),
                }),
                *span,
            )
        }
        Expr::Let(let_expr, span) => {
            use shape_ast::ast::expr_helpers::LetExpr;
            Expr::Let(
                Box::new(LetExpr {
                    pattern: let_expr.pattern.clone(),
                    type_annotation: let_expr.type_annotation.clone(),
                    value: let_expr.value.as_ref().map(|v| rec_box(v)),
                    body: rec_box(&let_expr.body),
                }),
                *span,
            )
        }
        Expr::Match(match_expr, span) => {
            use shape_ast::ast::expr_helpers::{MatchArm, MatchExpr};
            Expr::Match(
                Box::new(MatchExpr {
                    scrutinee: rec_box(&match_expr.scrutinee),
                    arms: match_expr
                        .arms
                        .iter()
                        .map(|arm| MatchArm {
                            pattern: arm.pattern.clone(),
                            guard: arm.guard.as_ref().map(|g| rec_box(g)),
                            body: rec_box(&arm.body),
                            pattern_span: arm.pattern_span,
                        })
                        .collect(),
                }),
                *span,
            )
        }
        Expr::Break(value, span) => Expr::Break(value.as_ref().map(|e| rec_box(e)), *span),
        Expr::TryOperator(inner, span) => Expr::TryOperator(rec_box(inner), *span),
        Expr::Await(inner, span) => Expr::Await(rec_box(inner), *span),
        Expr::AsyncScope(inner, span) => Expr::AsyncScope(rec_box(inner), *span),
        Expr::Spread(inner, span) => Expr::Spread(rec_box(inner), *span),
        // Everything else passes through verbatim — call expressions of the
        // closure parameter cannot appear in AST positions we don't traverse
        // here. Extend this match if additional shapes appear in stdlib
        // higher-order bodies.
        other => other.clone(),
    }
}

/// Build the inlined block that replaces `f(a1, a2, ...)`: a block with a
/// `let` prelude binding each closure formal to its corresponding arg, then
/// the closure body. Implemented via `Expr::Block` (a block expression) so it
/// fits cleanly into the replacement position.
fn build_inlined_closure_block(
    closure_params: &[String],
    call_args: &[shape_ast::ast::Expr],
    closure_body: &[shape_ast::ast::Statement],
) -> shape_ast::ast::Expr {
    use shape_ast::ast::expr_helpers::{BlockExpr, BlockItem};
    use shape_ast::ast::{DestructurePattern, Expr, Statement, VarKind, VariableDecl};

    let span = shape_ast::ast::Span::default();

    let mut items: Vec<BlockItem> = Vec::new();
    // Bind each closure formal to its corresponding arg via a let statement.
    for (pname, aexpr) in closure_params.iter().zip(call_args.iter()) {
        let decl = VariableDecl {
            kind: VarKind::Let,
            is_mut: false,
            pattern: DestructurePattern::Identifier(pname.clone(), span),
            type_annotation: None,
            value: Some(aexpr.clone()),
            ownership: Default::default(),
        };
        items.push(BlockItem::Statement(Statement::VariableDecl(decl, span)));
    }
    // cluster-2-cw-2-phaseC-inlining (2026-05-16): preserve the closure
    // body's tail-expression value semantics AND prevent in-body
    // `Statement::Return(...)` from becoming a function-level return of
    // the OUTER (monomorphized stdlib template) function.
    //
    // The arrow-function / pipe-lambda parser at
    // `crates/shape-ast/src/parser/expressions/functions.rs:62-64` /
    // `:134-135` wraps an expression-form closure body
    // (`|x| x * 2` / `x => x + 1`) as `vec![Statement::Return(Some(expr),
    // Span::DUMMY)]`. Inlining that statement verbatim into the
    // specialized body via `BlockItem::Statement(stmt.clone())` would
    // emit a MIR `lower_return_control_flow` → `Assign(SlotId(0),
    // body_value) + TerminatorKind::Return` inside the for-loop body of
    // the specialized fn, returning the FIRST iteration's mapped value
    // instead of pushing it.
    //
    // Fix: rewrite a trailing `Statement::Return(Some(expr), _)` AND
    // `Statement::Expression(expr, _)` as `BlockItem::Expression(expr)`
    // so the block evaluates to the expression's value and the OUTER
    // method-call arg receives that value, without emitting a
    // function-level return. Empirically dispositioned at HEAD
    // ca8300f0 via the `[phaseC-empirical]` trace + smoke-2 VM
    // observation of `no method 'sum' on receiver kind Int64`.
    let last_idx = closure_body.len().saturating_sub(1);
    for (i, stmt) in closure_body.iter().enumerate() {
        if i == last_idx {
            match stmt {
                shape_ast::ast::Statement::Expression(expr, _) => {
                    items.push(BlockItem::Expression(expr.clone()));
                    continue;
                }
                shape_ast::ast::Statement::Return(Some(expr), _) => {
                    items.push(BlockItem::Expression(expr.clone()));
                    continue;
                }
                _ => {}
            }
        }
        items.push(BlockItem::Statement(stmt.clone()));
    }

    Expr::Block(BlockExpr { items }, span)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::expressions::Expr;
    use shape_ast::ast::functions::FunctionParameter;
    use shape_ast::ast::patterns::DestructurePattern;
    use shape_ast::ast::span::Span;
    use shape_ast::ast::statements::Statement;
    use shape_ast::ast::types::{TypeAnnotation, TypeParam};

    fn ident_param(name: &str, ty: TypeAnnotation) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier(name.into(), Span::default()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(ty),
            default_value: None,
        }
    }

    fn type_param(name: &str) -> TypeParam {
        TypeParam::Type {
            name: name.into(),
            span: Span::default(),
            doc_comment: None,
            default_type: None,
            trait_bounds: vec![],
        }
    }

    fn ref_t(name: &str) -> TypeAnnotation {
        TypeAnnotation::Reference(TypePath::simple(name))
    }

    /// Build a no-op identity function: `fn id<T>(x: T) -> T { return x }`.
    fn identity_fn() -> FunctionDef {
        FunctionDef {
            name: "id".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: Some(vec![type_param("T")]),
            params: vec![ident_param("x", ref_t("T"))],
            return_type: Some(ref_t("T")),
            where_clause: None,
            body: vec![Statement::Return(
                Some(Expr::Identifier("x".into(), Span::default())),
                Span::default(),
            )],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        }
    }

    #[test]
    fn concrete_to_annotation_primitives() {
        assert_eq!(
            concrete_to_annotation(&ConcreteType::F64),
            TypeAnnotation::Basic("number".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::I64),
            TypeAnnotation::Basic("int".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::Bool),
            TypeAnnotation::Basic("bool".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::String),
            TypeAnnotation::Basic("string".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::I32),
            TypeAnnotation::Basic("i32".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::U8),
            TypeAnnotation::Basic("u8".into())
        );
        assert_eq!(
            concrete_to_annotation(&ConcreteType::Void),
            TypeAnnotation::Void
        );
    }

    #[test]
    fn concrete_to_annotation_composites() {
        let arr = ConcreteType::Array(Box::new(ConcreteType::F64));
        match concrete_to_annotation(&arr) {
            TypeAnnotation::Generic { name, args } => {
                assert_eq!(name.as_str(), "Array");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], TypeAnnotation::Basic("number".into()));
            }
            other => panic!("expected Generic, got {:?}", other),
        }

        let map = ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        );
        match concrete_to_annotation(&map) {
            TypeAnnotation::Generic { name, args } => {
                assert_eq!(name.as_str(), "HashMap");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], TypeAnnotation::Basic("string".into()));
                assert_eq!(args[1], TypeAnnotation::Basic("int".into()));
            }
            other => panic!("expected Generic, got {:?}", other),
        }

        let tup = ConcreteType::Tuple(vec![ConcreteType::I64, ConcreteType::F64]);
        match concrete_to_annotation(&tup) {
            TypeAnnotation::Tuple(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], TypeAnnotation::Basic("int".into()));
                assert_eq!(items[1], TypeAnnotation::Basic("number".into()));
            }
            other => panic!("expected Tuple, got {:?}", other),
        }
    }

    #[test]
    fn substitute_simple_reference() {
        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::I64);

        let ann = ref_t("T");
        let out = substitute_type_annotation(&ann, &subs);
        assert_eq!(out, TypeAnnotation::Basic("int".into()));

        // A non-substituted reference is left alone.
        let other = ref_t("MyType");
        let out2 = substitute_type_annotation(&other, &subs);
        assert_eq!(out2, ref_t("MyType"));
    }

    #[test]
    fn substitute_does_not_touch_qualified_paths() {
        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::I64);

        // `mod::T` is not a type parameter — it's a qualified type ref.
        let qualified =
            TypeAnnotation::Reference(TypePath::from_segments(vec!["mod".into(), "T".into()]));
        let out = substitute_type_annotation(&qualified, &subs);
        assert_eq!(out, qualified);
    }

    #[test]
    fn substitute_nested_array_of_t() {
        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::I64);

        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![ref_t("T")],
        };
        let out = substitute_type_annotation(&ann, &subs);
        match out {
            TypeAnnotation::Generic { name, args } => {
                assert_eq!(name.as_str(), "Array");
                assert_eq!(args, vec![TypeAnnotation::Basic("int".into())]);
            }
            other => panic!("expected Generic, got {:?}", other),
        }
    }

    #[test]
    fn substitute_function_t_to_i64() {
        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::I64);

        let func = identity_fn();
        let mono = substitute_function_def(&func, &subs);

        // Param type is now `int`.
        assert_eq!(mono.params.len(), 1);
        assert_eq!(
            mono.params[0].type_annotation,
            Some(TypeAnnotation::Basic("int".into()))
        );

        // Return type is `int`.
        assert_eq!(
            mono.return_type,
            Some(TypeAnnotation::Basic("int".into()))
        );

        // Generics dropped.
        assert!(mono.type_params.is_none());

        // Name carries the mono key suffix.
        assert_eq!(mono.name, "id::i64");
    }

    #[test]
    fn substitute_map_t_u_to_number_string() {
        // fn map<T, U>(arr: Array<T>, f: (T) -> U) -> Array<U>
        let func = FunctionDef {
            name: "map".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: Some(vec![type_param("T"), type_param("U")]),
            params: vec![
                ident_param(
                    "arr",
                    TypeAnnotation::Generic {
                        name: TypePath::simple("Array"),
                        args: vec![ref_t("T")],
                    },
                ),
                ident_param(
                    "f",
                    TypeAnnotation::Function {
                        params: vec![FunctionParam {
                            name: None,
                            optional: false,
                            type_annotation: ref_t("T"),
                        }],
                        returns: Box::new(ref_t("U")),
                    },
                ),
            ],
            return_type: Some(TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![ref_t("U")],
            }),
            where_clause: None,
            body: vec![],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };

        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::F64);
        subs.insert("U".to_string(), ConcreteType::String);

        let mono = substitute_function_def(&func, &subs);

        // arr: Array<number>
        match &mono.params[0].type_annotation {
            Some(TypeAnnotation::Generic { name, args }) => {
                assert_eq!(name.as_str(), "Array");
                assert_eq!(args, &vec![TypeAnnotation::Basic("number".into())]);
            }
            other => panic!("expected Generic Array, got {:?}", other),
        }

        // f: (number) -> string
        match &mono.params[1].type_annotation {
            Some(TypeAnnotation::Function { params, returns }) => {
                assert_eq!(params.len(), 1);
                assert_eq!(
                    params[0].type_annotation,
                    TypeAnnotation::Basic("number".into())
                );
                assert_eq!(**returns, TypeAnnotation::Basic("string".into()));
            }
            other => panic!("expected Function, got {:?}", other),
        }

        // -> Array<string>
        match &mono.return_type {
            Some(TypeAnnotation::Generic { name, args }) => {
                assert_eq!(name.as_str(), "Array");
                assert_eq!(args, &vec![TypeAnnotation::Basic("string".into())]);
            }
            other => panic!("expected Generic Array, got {:?}", other),
        }

        // Mono key is sorted by type-param name → T then U → "f64_string".
        assert_eq!(mono.name, "map::f64_string");
    }

    #[test]
    fn substitute_let_with_array_of_t_in_body() {
        // fn pack<T>(x: T) { let arr: Array<T> = []; return arr; }
        let func = FunctionDef {
            name: "pack".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: Some(vec![type_param("T")]),
            params: vec![ident_param("x", ref_t("T"))],
            return_type: None,
            where_clause: None,
            body: vec![
                Statement::VariableDecl(
                    shape_ast::ast::program::VariableDecl {
                        kind: shape_ast::ast::program::VarKind::Let,
                        is_mut: false,
                        pattern: DestructurePattern::Identifier("arr".into(), Span::default()),
                        type_annotation: Some(TypeAnnotation::Generic {
                            name: TypePath::simple("Array"),
                            args: vec![ref_t("T")],
                        }),
                        value: Some(Expr::Array(vec![], Span::default())),
                        ownership: Default::default(),
                    },
                    Span::default(),
                ),
                Statement::Return(
                    Some(Expr::Identifier("arr".into(), Span::default())),
                    Span::default(),
                ),
            ],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };

        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::I64);

        let mono = substitute_function_def(&func, &subs);

        // The let in the body should now be `let arr: Array<int> = []`.
        match &mono.body[0] {
            Statement::VariableDecl(decl, _) => match &decl.type_annotation {
                Some(TypeAnnotation::Generic { name, args }) => {
                    assert_eq!(name.as_str(), "Array");
                    assert_eq!(args, &vec![TypeAnnotation::Basic("int".into())]);
                }
                other => panic!("expected Array<int>, got {:?}", other),
            },
            other => panic!("expected VariableDecl, got {:?}", other),
        }
    }

    #[test]
    fn mono_key_is_deterministic() {
        let mut a = HashMap::new();
        a.insert("U".to_string(), ConcreteType::String);
        a.insert("T".to_string(), ConcreteType::F64);

        let mut b = HashMap::new();
        b.insert("T".to_string(), ConcreteType::F64);
        b.insert("U".to_string(), ConcreteType::String);

        // Same substitution map, two different insertion orders.
        assert_eq!(mono_key_from_subs(&a), mono_key_from_subs(&b));
        // Sorted by key — T (f64) before U (string).
        assert_eq!(mono_key_from_subs(&a), "f64_string");
    }

    #[test]
    fn cloned_function_name_has_mono_suffix() {
        let mut subs = HashMap::new();
        subs.insert("T".to_string(), ConcreteType::Bool);

        let func = identity_fn();
        let mono = substitute_function_def(&func, &subs);
        assert!(
            mono.name.starts_with("id::"),
            "expected name to start with 'id::', got {}",
            mono.name
        );
        assert_eq!(mono.name, "id::bool");
    }

    // ---- Const generic substitution tests --------------------------------

    /// Build a synthetic `repeat` function as if the grammar had support for
    /// `fn repeat<const N: int>(x: number) -> Array<number>`. Until the
    /// grammar lands we model the const param as a normal `number` value
    /// inside the body — the substitution path replaces it with a literal.
    fn repeat_fn() -> FunctionDef {
        FunctionDef {
            name: "repeat".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            // No type params — repeat has only a const generic, which the
            // grammar doesn't model yet so this stays empty.
            type_params: None,
            params: vec![ident_param("x", TypeAnnotation::Basic("number".into()))],
            return_type: Some(TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Basic("number".into())],
            }),
            where_clause: None,
            // Body: `return __const_0;` — the synthesised name we use for the
            // first const generic param. The substitution pass will rewrite
            // it to `return 3;` for `N = 3`.
            body: vec![Statement::Return(
                Some(Expr::Identifier("__const_0".into(), Span::default())),
                Span::default(),
            )],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        }
    }

    #[test]
    fn substitute_with_consts_renames_function() {
        let func = repeat_fn();
        let type_subs: HashMap<String, ConcreteType> = HashMap::new();
        let mut const_subs: HashMap<String, ComptimeConstValue> = HashMap::new();
        const_subs.insert("__const_0".into(), ComptimeConstValue::Int(3));

        let mono = substitute_function_def_with_consts(
            &func,
            &type_subs,
            &const_subs,
            "repeat::int_3",
        );

        // Name follows the `<base>::<suffix>` convention.
        assert_eq!(mono.name, "repeat::int_3");
        assert!(mono.type_params.is_none());
    }

    #[test]
    fn substitute_with_consts_replaces_identifier_with_literal() {
        let func = repeat_fn();
        let type_subs: HashMap<String, ConcreteType> = HashMap::new();
        let mut const_subs: HashMap<String, ComptimeConstValue> = HashMap::new();
        const_subs.insert("__const_0".into(), ComptimeConstValue::Int(3));

        let mono = substitute_function_def_with_consts(
            &func,
            &type_subs,
            &const_subs,
            "repeat::int_3",
        );

        // Body should now be `return 3` instead of `return __const_0`.
        match &mono.body[0] {
            Statement::Return(Some(Expr::Literal(lit, _)), _) => match lit {
                shape_ast::ast::Literal::Int(3) => {}
                other => panic!("expected Literal::Int(3), got {:?}", other),
            },
            other => panic!("expected return with int literal, got {:?}", other),
        }
    }

    #[test]
    fn substitute_with_consts_two_distinct_values_produce_distinct_bodies() {
        let func = repeat_fn();
        let type_subs: HashMap<String, ConcreteType> = HashMap::new();

        let mut subs_3: HashMap<String, ComptimeConstValue> = HashMap::new();
        subs_3.insert("__const_0".into(), ComptimeConstValue::Int(3));
        let mut subs_5: HashMap<String, ComptimeConstValue> = HashMap::new();
        subs_5.insert("__const_0".into(), ComptimeConstValue::Int(5));

        let mono_3 =
            substitute_function_def_with_consts(&func, &type_subs, &subs_3, "repeat::int_3");
        let mono_5 =
            substitute_function_def_with_consts(&func, &type_subs, &subs_5, "repeat::int_5");

        assert_ne!(mono_3.name, mono_5.name);
        assert_eq!(mono_3.name, "repeat::int_3");
        assert_eq!(mono_5.name, "repeat::int_5");

        // And the bodies must reflect the bound values.
        let extract_int = |def: &FunctionDef| -> i64 {
            match &def.body[0] {
                Statement::Return(Some(Expr::Literal(shape_ast::ast::Literal::Int(i), _)), _) => *i,
                other => panic!("unexpected body shape: {:?}", other),
            }
        };
        assert_eq!(extract_int(&mono_3), 3);
        assert_eq!(extract_int(&mono_5), 5);
    }

    #[test]
    fn substitute_with_consts_empty_const_subs_is_identity_on_body() {
        // With an empty const_subs map, the body must be byte-identical to
        // the input — the const substitution pass is a no-op.
        let func = repeat_fn();
        let type_subs: HashMap<String, ConcreteType> = HashMap::new();
        let const_subs: HashMap<String, ComptimeConstValue> = HashMap::new();

        let mono = substitute_function_def_with_consts(
            &func,
            &type_subs,
            &const_subs,
            "repeat",
        );

        // Body identifier is preserved as-is.
        match &mono.body[0] {
            Statement::Return(Some(Expr::Identifier(name, _)), _) => {
                assert_eq!(name, "__const_0");
            }
            other => panic!("expected identifier return, got {:?}", other),
        }
    }

    #[test]
    fn substitute_with_consts_combined_with_type_substitution() {
        // Build a function `fn matrix<T>(x: T) -> T { return __const_0; }`
        // where __const_0 stands in for an `int`-typed const generic ROWS=4.
        // The substitution should:
        //   - rewrite the parameter type from T to f64,
        //   - rewrite the return type from T to f64,
        //   - rewrite the body identifier `__const_0` to `4`.
        let func = FunctionDef {
            name: "matrix".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: Some(vec![type_param("T")]),
            params: vec![ident_param("x", ref_t("T"))],
            return_type: Some(ref_t("T")),
            where_clause: None,
            body: vec![Statement::Return(
                Some(Expr::Identifier("__const_0".into(), Span::default())),
                Span::default(),
            )],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };

        let mut type_subs: HashMap<String, ConcreteType> = HashMap::new();
        type_subs.insert("T".into(), ConcreteType::F64);

        let mut const_subs: HashMap<String, ComptimeConstValue> = HashMap::new();
        const_subs.insert("__const_0".into(), ComptimeConstValue::Int(4));

        let mono = substitute_function_def_with_consts(
            &func,
            &type_subs,
            &const_subs,
            "matrix::f64_int_4",
        );

        assert_eq!(mono.name, "matrix::f64_int_4");
        assert_eq!(
            mono.params[0].type_annotation,
            Some(TypeAnnotation::Basic("number".into()))
        );
        assert_eq!(
            mono.return_type,
            Some(TypeAnnotation::Basic("number".into()))
        );
        match &mono.body[0] {
            Statement::Return(Some(Expr::Literal(shape_ast::ast::Literal::Int(4), _)), _) => {}
            other => panic!("expected Int(4) literal in body, got {:?}", other),
        }
    }

    // =====================================================================
    // Phase C — inline_closure_body_into_specialization tests.
    // =====================================================================

    /// `f(item)` inside a return statement is replaced by the inlined
    /// closure body wrapped in a block that binds the closure's formal
    /// params.
    #[test]
    fn phase_c_inline_closure_body_replaces_call_in_return() {
        // Specialized body: `fn map::i64(self: Array<int>, f: (int) => int) { return f(item) }`
        let mut spec = FunctionDef {
            name: "map::i64".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![
                ident_param("self", ref_t("Array")),
                ident_param("f", ref_t("fn_type")),
            ],
            return_type: None,
            where_clause: None,
            body: vec![Statement::Return(
                Some(Expr::FunctionCall {
                    name: "f".into(),
                    args: vec![Expr::Identifier("item".into(), Span::default())],
                    named_args: vec![],
                    span: Span::default(),
                }),
                Span::default(),
            )],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };
        // Closure: `|x| x + 1`
        let closure_body = vec![Statement::Expression(
            Expr::BinaryOp {
                left: Box::new(Expr::Identifier("x".into(), Span::default())),
                op: shape_ast::ast::BinaryOp::Add,
                right: Box::new(Expr::Literal(
                    shape_ast::ast::Literal::Int(1),
                    Span::default(),
                )),
                span: Span::default(),
            },
            Span::default(),
        )];

        super::inline_closure_body_into_specialization(
            &mut spec,
            "f",
            &["x".into()],
            &closure_body,
            &[], // no captures
        )
        .expect("inlining should succeed");

        // Body should no longer contain a FunctionCall named "f".
        match &spec.body[0] {
            Statement::Return(Some(Expr::Block(block, _)), _) => {
                // Block items: one VariableDecl (let x = item), one Expression (x + 1).
                assert_eq!(block.items.len(), 2);
            }
            other => panic!("expected Return(Block(..)), got {:?}", other),
        }
    }

    /// Phase C inlining preserves the specialized function's `name`
    /// unchanged — it's the caller's responsibility to rename.
    #[test]
    fn phase_c_inline_preserves_specialization_name() {
        let mut spec = FunctionDef {
            name: "original_name".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![ident_param("f", ref_t("fn_type"))],
            return_type: None,
            where_clause: None,
            body: vec![],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };
        super::inline_closure_body_into_specialization(
            &mut spec,
            "f",
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(spec.name, "original_name");
    }

    /// Inlining KEEPS the formal closure parameter in the specialized
    /// body's param list — Phase C intentionally doesn't strip it so the
    /// call-site ABI stays unchanged. Capture hoisting is Phase D/E work.
    #[test]
    fn phase_c_inline_preserves_formal_closure_param() {
        let mut spec = FunctionDef {
            name: "map::i64_closure_0_i64".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![
                ident_param("self", ref_t("Array")),
                ident_param("f", ref_t("fn_type")),
            ],
            return_type: None,
            where_clause: None,
            body: vec![],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };
        super::inline_closure_body_into_specialization(
            &mut spec,
            "f",
            &["x".into()],
            &[],
            &["captured_n".into()], // unused by Phase C
        )
        .unwrap();
        // Params should still contain `self` AND `f` (the closure formal).
        let names: Vec<String> = spec
            .params
            .iter()
            .flat_map(|p| p.get_identifiers())
            .collect();
        assert!(
            names.contains(&"self".to_string()),
            "expected self in params: {:?}",
            names
        );
        assert!(
            names.contains(&"f".to_string()),
            "expected formal closure param `f` preserved: {:?}",
            names
        );
    }

    /// Non-matching FunctionCalls (e.g. calls to other named functions) are
    /// passed through unchanged by the inliner.
    #[test]
    fn phase_c_inline_leaves_unrelated_calls_alone() {
        let mut spec = FunctionDef {
            name: "map".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![ident_param("f", ref_t("fn_type"))],
            return_type: None,
            where_clause: None,
            body: vec![Statement::Return(
                Some(Expr::FunctionCall {
                    name: "println".into(), // not the closure param
                    args: vec![Expr::Literal(
                        shape_ast::ast::Literal::String("hi".into()),
                        Span::default(),
                    )],
                    named_args: vec![],
                    span: Span::default(),
                }),
                Span::default(),
            )],
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        };
        super::inline_closure_body_into_specialization(
            &mut spec,
            "f",
            &[],
            &[],
            &[],
        )
        .unwrap();
        // The `println(...)` call should be intact — its name was not `f`.
        match &spec.body[0] {
            Statement::Return(Some(Expr::FunctionCall { name, .. }), _) => {
                assert_eq!(name, "println");
            }
            other => panic!("expected FunctionCall(println) unchanged, got {:?}", other),
        }
    }

    // =====================================================================
    // B.4 — broad-coverage const substitution tests
    //
    // These pin behaviour for statement / expression variants that prior to
    // B.4 passed through unchanged. Each test constructs a minimal body
    // containing `__const_0` (or `__const_1`) in a specific AST position,
    // and asserts the post-substitution body no longer contains the
    // identifier.
    // =====================================================================

    /// Helper: build a function whose body is `body`, no type params, no
    /// return type, no params. Enough for subst-path tests that don't need
    /// type checking downstream.
    fn const_body_fn(body: Vec<Statement>) -> FunctionDef {
        FunctionDef {
            name: "harness".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![],
            return_type: None,
            where_clause: None,
            body,
            annotations: vec![],
            is_async: false,
            is_comptime: false,
        }
    }

    fn const_subs_int_0(v: i64) -> HashMap<String, ComptimeConstValue> {
        let mut m: HashMap<String, ComptimeConstValue> = HashMap::new();
        m.insert("__const_0".into(), ComptimeConstValue::Int(v));
        m
    }

    /// Walk an entire function body and assert NO `Expr::Identifier("__const_0", _)`
    /// survives anywhere. Used as the invariant for each new test.
    fn assert_no_const_id_in_stmt(stmt: &Statement) {
        match stmt {
            Statement::Return(expr, _) => {
                if let Some(e) = expr {
                    assert_no_const_id_in_expr(e);
                }
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => {}
            Statement::VariableDecl(decl, _) => {
                if let Some(v) = &decl.value {
                    assert_no_const_id_in_expr(v);
                }
            }
            Statement::Assignment(a, _) => assert_no_const_id_in_expr(&a.value),
            Statement::Expression(e, _) => assert_no_const_id_in_expr(e),
            Statement::For(fl, _) => {
                match &fl.init {
                    ForInit::ForIn { iter, .. } => assert_no_const_id_in_expr(iter),
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        assert_no_const_id_in_stmt(init);
                        assert_no_const_id_in_expr(condition);
                        assert_no_const_id_in_expr(update);
                    }
                }
                for s in &fl.body {
                    assert_no_const_id_in_stmt(s);
                }
            }
            Statement::While(wl, _) => {
                assert_no_const_id_in_expr(&wl.condition);
                for s in &wl.body {
                    assert_no_const_id_in_stmt(s);
                }
            }
            Statement::If(is, _) => {
                assert_no_const_id_in_expr(&is.condition);
                for s in &is.then_body {
                    assert_no_const_id_in_stmt(s);
                }
                if let Some(eb) = &is.else_body {
                    for s in eb {
                        assert_no_const_id_in_stmt(s);
                    }
                }
            }
            Statement::Extend(_, _) => {}
            Statement::SetParamType { .. } | Statement::SetReturnType { .. } => {}
            Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                assert_no_const_id_in_expr(expression)
            }
            Statement::ReplaceBody { body, .. } => {
                for s in body {
                    assert_no_const_id_in_stmt(s);
                }
            }
        }
    }

    fn assert_no_const_id_in_expr(expr: &Expr) {
        if let Expr::Identifier(name, _) = expr {
            assert_ne!(
                name, "__const_0",
                "residual const identifier in expression: {:?}",
                expr
            );
            return;
        }
        // For other expressions, recurse through a best-effort walker that
        // mirrors substitute_const_in_expr's recursion surface. Any variant
        // we don't explicitly walk here is a leaf for identifier purposes.
        match expr {
            Expr::BinaryOp { left, right, .. }
            | Expr::FuzzyComparison { left, right, .. } => {
                assert_no_const_id_in_expr(left);
                assert_no_const_id_in_expr(right);
            }
            Expr::UnaryOp { operand, .. } => assert_no_const_id_in_expr(operand),
            Expr::Array(items, _) => items.iter().for_each(assert_no_const_id_in_expr),
            Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => {
                args.iter().for_each(assert_no_const_id_in_expr);
            }
            Expr::MethodCall { receiver, args, .. } => {
                assert_no_const_id_in_expr(receiver);
                args.iter().for_each(assert_no_const_id_in_expr);
            }
            Expr::IndexAccess { object, index, .. } => {
                assert_no_const_id_in_expr(object);
                assert_no_const_id_in_expr(index);
            }
            Expr::PropertyAccess { object, .. } => assert_no_const_id_in_expr(object),
            Expr::If(ie, _) => {
                assert_no_const_id_in_expr(&ie.condition);
                assert_no_const_id_in_expr(&ie.then_branch);
                if let Some(eb) = &ie.else_branch {
                    assert_no_const_id_in_expr(eb);
                }
            }
            Expr::Match(me, _) => {
                assert_no_const_id_in_expr(&me.scrutinee);
                for arm in &me.arms {
                    assert_no_const_id_in_expr(&arm.body);
                }
            }
            Expr::Block(block, _) => {
                for it in &block.items {
                    match it {
                        BlockItem::Expression(e) => assert_no_const_id_in_expr(e),
                        BlockItem::VariableDecl(d) => {
                            if let Some(v) = &d.value {
                                assert_no_const_id_in_expr(v);
                            }
                        }
                        BlockItem::Assignment(a) => assert_no_const_id_in_expr(&a.value),
                        BlockItem::Statement(s) => assert_no_const_id_in_stmt(s),
                    }
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for s in body {
                    assert_no_const_id_in_stmt(s);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(s) = start {
                    assert_no_const_id_in_expr(s);
                }
                if let Some(e) = end {
                    assert_no_const_id_in_expr(e);
                }
            }
            _ => {}
        }
    }

    /// Identifier inside an `if` condition is substituted.
    #[test]
    fn b4_const_in_if_statement() {
        let stmt = Statement::If(
            IfStatement {
                condition: Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                    op: shape_ast::ast::BinaryOp::Equal,
                    right: Box::new(Expr::Literal(
                        shape_ast::ast::Literal::Int(1),
                        Span::default(),
                    )),
                    span: Span::default(),
                },
                then_body: vec![Statement::Return(
                    Some(Expr::Identifier("__const_0".into(), Span::default())),
                    Span::default(),
                )],
                else_body: Some(vec![Statement::Expression(
                    Expr::Identifier("__const_0".into(), Span::default()),
                    Span::default(),
                )]),
            },
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(7),
            "harness::int_7",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside a `while` loop condition and body is substituted.
    #[test]
    fn b4_const_in_while_statement() {
        let stmt = Statement::While(
            WhileLoop {
                condition: Expr::Identifier("__const_0".into(), Span::default()),
                body: vec![Statement::Return(
                    Some(Expr::Identifier("__const_0".into(), Span::default())),
                    Span::default(),
                )],
            },
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(2),
            "harness::int_2",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside a `for` loop body (for-in shape) is substituted.
    #[test]
    fn b4_const_in_for_statement() {
        use shape_ast::ast::statements::ForLoop;
        let stmt = Statement::For(
            ForLoop {
                init: ForInit::ForIn {
                    pattern: DestructurePattern::Identifier("i".into(), Span::default()),
                    iter: Expr::Identifier("__const_0".into(), Span::default()),
                },
                body: vec![Statement::Expression(
                    Expr::Identifier("__const_0".into(), Span::default()),
                    Span::default(),
                )],
                is_async: false,
            },
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(4),
            "harness::int_4",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside an assignment RHS is substituted.
    #[test]
    fn b4_const_in_assignment_rhs() {
        use shape_ast::ast::program::Assignment;
        let stmt = Statement::Assignment(
            Assignment {
                pattern: DestructurePattern::Identifier("x".into(), Span::default()),
                value: Expr::Identifier("__const_0".into(), Span::default()),
            },
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(11),
            "harness::int_11",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside a method call receiver / args is substituted.
    #[test]
    fn b4_const_in_method_call() {
        let stmt = Statement::Expression(
            Expr::MethodCall {
                receiver: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                method: "to_string".into(),
                args: vec![Expr::Identifier("__const_0".into(), Span::default())],
                named_args: vec![],
                optional: false,
                span: Span::default(),
            },
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(9),
            "harness::int_9",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside a closure body (Expr::FunctionExpr) is substituted.
    /// This is the optional plan test — closures substitute transparently.
    #[test]
    fn b4_const_in_closure_body() {
        let closure = Expr::FunctionExpr {
            params: vec![ident_param("x", ref_t("int"))],
            return_type: None,
            body: vec![Statement::Return(
                Some(Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".into(), Span::default())),
                    op: shape_ast::ast::BinaryOp::Mul,
                    right: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                    span: Span::default(),
                }),
                Span::default(),
            )],
            span: Span::default(),
        };
        let stmt = Statement::Expression(closure, Span::default());
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(5),
            "harness::int_5",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Identifier inside a match arm body / guard is substituted.
    #[test]
    fn b4_const_in_match_arm() {
        use shape_ast::ast::expr_helpers::{MatchArm, MatchExpr};
        use shape_ast::ast::patterns::Pattern;
        let stmt = Statement::Expression(
            Expr::Match(
                Box::new(MatchExpr {
                    scrutinee: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                    arms: vec![MatchArm {
                        pattern: Pattern::Wildcard,
                        guard: Some(Box::new(Expr::Identifier(
                            "__const_0".into(),
                            Span::default(),
                        ))),
                        body: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                        pattern_span: Some(Span::default()),
                    }],
                }),
                Span::default(),
            ),
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);
        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &const_subs_int_0(3),
            "harness::int_3",
        );
        assert_no_const_id_in_stmt(&mono.body[0]);
    }

    /// Multiple const params (e.g. Matrix<R, C>) — each binds to its own value.
    #[test]
    fn b4_multi_const_params_substitute_distinctly() {
        let stmt = Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Identifier("__const_0".into(), Span::default())),
                op: shape_ast::ast::BinaryOp::Mul,
                right: Box::new(Expr::Identifier("__const_1".into(), Span::default())),
                span: Span::default(),
            }),
            Span::default(),
        );
        let func = const_body_fn(vec![stmt]);

        let mut subs: HashMap<String, ComptimeConstValue> = HashMap::new();
        subs.insert("__const_0".into(), ComptimeConstValue::Int(4));
        subs.insert("__const_1".into(), ComptimeConstValue::Int(3));

        let mono = substitute_function_def_with_consts(
            &func,
            &HashMap::new(),
            &subs,
            "harness::int_4_int_3",
        );

        // Body should be `return 4 * 3` — both identifiers rewritten.
        match &mono.body[0] {
            Statement::Return(Some(Expr::BinaryOp { left, right, .. }), _) => {
                assert!(matches!(
                    left.as_ref(),
                    Expr::Literal(shape_ast::ast::Literal::Int(4), _)
                ));
                assert!(matches!(
                    right.as_ref(),
                    Expr::Literal(shape_ast::ast::Literal::Int(3), _)
                ));
            }
            other => panic!("expected `return 4 * 3`, got {:?}", other),
        }
    }
}
