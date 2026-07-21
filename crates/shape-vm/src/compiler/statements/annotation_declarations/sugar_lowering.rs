//! ADR-009 C3 #14 (slice 4, S4c) — THE SUGAR LOWERING onto the public API.
//!
//! A TypedConfig annotation's declarative `before`/`after` block lowers
//! COMPILER-SIDE onto exactly what the public comptime hook-template API
//! expresses (C3-G2, ZERO private side-channels BY CONSTRUCTION):
//!
//!   1. Each declarative hook becomes a MINTED module-scope-shaped body fn
//!      (C3-G3 — hook bodies are ordinary typed Shape functions):
//!        - `before(args)` (exactly one param) → the POLYMORPHIC form
//!          `fn <minted><Args>(args: Args, <config params in declared order
//!          with declared annotations>) -> Args { <handler body verbatim> }`
//!          — the body must `return args`; the lowering NEVER auto-appends a
//!          return (the S1/S2 classifier's named rejections fire on wrong
//!          shapes exactly as they do for hand-written API bodies).
//!        - `after(result)` → `fn <minted><R>(result: R, <config>) -> R`.
//!        - `before()` / `after()` (zero params) → the F1 concrete OBSERVER
//!          form `fn <minted>(<config>) { body }`.
//!      Config params are appended as TRAILING CAPTURE PARAMS with their
//!      DECLARED annotations verbatim (C3-G4/G5: config enters the body ONLY
//!      as ConstLift'd declared captures — the mixed-type shape S4a's generic
//!      `capture<T>` forwarder unlocked).
//!   2. ONE additional `comptime post`-shaped handler is SYNTHESIZED whose
//!      body is EXACTLY the public-API program — for each declarative hook in
//!      declaration order:
//!        `install(before_hook(<minted_ident>, [capture("p1", p1), …]))`
//!      (or `after_hook`), the captures spelling every config param in
//!      declared order (the S2 trailing-param name bijection). No other
//!      statements; no directive pushes; no store writes. Every capability
//!      the lowering uses is public-API-expressible — the S2d/S4b sugar
//!      matrix is the completeness proof, and the synthesized handler runs
//!      through the SAME executor path as a hand-written handler.
//!
//! Handler shapes outside the four typed-surface forms are the R3
//! declaration-site rejection (fired by the planner through this module's
//! single producer); `on_define`/`metadata` in a TypedConfig definition are
//! the R3-family rejection citing the E4/S6 fence. There is NO `ctx` on the
//! typed surface (S2 fix-round F3; E4 owns the runtime-hook ctx family) — a
//! body referencing `ctx` fails as an ordinary unresolved identifier, loud.
//!
//! DESIGN NOTE (recorded per the stage charter): declarative handlers lower
//! to the polymorphic/observer forms ONLY this slice; CONCRETE hook bodies
//! stay API-spelled (`install(before_hook(my_concrete_fn, …))` inside a
//! hand-written comptime handler).
//!
//! MINTING/HYGIENE: body-fn names and the polymorphic type-param name are
//! HYGIENIC (`mint_hygienic_fn_name_stable`, SOH-prefixed unspellable
//! descriptors) — user shadowing is structurally impossible, so the Lens-2
//! F4 shadow tracking never engages for synthesized handlers, and the minted
//! generic param can never capture a user nominal type spelled inside the
//! verbatim hook body. The nonce is a STABLE digest of (definition name,
//! hook kind, handler index): one definition mints ONE identity per hook
//! across every handler run, which is what lets Dec-95 rule 6 SHARE a baked
//! specialization across two applications with equal config. (The LocalAst
//! pre-pass provenance derives its own artifacts from the local AST def;
//! pre-pass installs are no-op APPLY arms, so only the pass-2 artifacts —
//! stored on `CompiledAnnotation` by the installer — carry identity.)

use shape_ast::ast::{
    AnnotationDef, AnnotationHandler, AnnotationHandlerParam, AnnotationHandlerType,
    AnnotationTargetKind, BlockExpr, BlockItem, DestructurePattern, Expr, FunctionDef,
    FunctionParameter, Literal, Span, Spanned, Statement, TypeAnnotation, TypeParam,
};

use crate::compiler::{BytecodeCompiler, HygienicRole};

/// The lowering's artifacts for ONE TypedConfig definition with declarative
/// hooks: the synthesized public-API `comptime post` handler plus the minted
/// body fns it references. Stored on `CompiledAnnotation`
/// (`sugar_post_handler` / `sugar_body_fns`, both serde-skip beside the
/// comptime handlers) and threaded into `ComptimeAnnotationHandlers` by both
/// handler-resolution provenances.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct SugarLowering {
    pub(in crate::compiler) post_handler: AnnotationHandler,
    pub(in crate::compiler) body_fns: Vec<FunctionDef>,
}

/// A declaration-site rejection from the lowering (R3 / R3-family), carrying
/// the offending handler's span for attribution. The planner converts it to
/// the spanned `SemanticError` — ONE firing site.
#[derive(Debug)]
pub(in crate::compiler) struct SugarLoweringRejection {
    pub(in crate::compiler) message: String,
    pub(in crate::compiler) span: Span,
}

/// THE lowering producer (single producer; planner + LocalAst handler
/// resolution both call it, the installer stores its output). Returns
/// `Ok(None)` for a TypedConfig definition with NO declarative hooks (the
/// hand-written-API spelling — nothing to lower).
pub(in crate::compiler) fn lower_typed_config_declarative_hooks(
    compiler: &BytecodeCompiler,
    definition: &AnnotationDef,
) -> Result<Option<SugarLowering>, SugarLoweringRejection> {
    let config_params = config_param_carrier(definition);

    let mut body_fns = Vec::new();
    let mut install_items = Vec::new();

    for (index, handler) in definition.handlers.iter().enumerate() {
        match &handler.handler_type {
            // Comptime handlers are classification-independent (they run
            // through the mini-VM on both surface classes) and may COEXIST
            // with declarative hooks — the synthesized handler is appended
            // AFTER them by the resolution seams.
            AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost => continue,
            // R3-family: lifecycle hooks have no typed-surface form yet.
            AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata => {
                return Err(SugarLoweringRejection {
                    message: format!(
                        "annotation `{}` declares typed config parameters, which selects the \
                         typed hook surface, but its `{}` handler is a runtime-lifecycle hook \
                         with no typed-surface form yet (the runtime-hook context family is \
                         E4's charter; the legacy lifecycle surface is deleted at C3-S6) — \
                         remove the parameter types to stay on the legacy surface until it is \
                         deleted (C3-G7/S6)",
                        definition.name,
                        hook_kind_word(&handler.handler_type),
                    ),
                    span: handler.span,
                });
            }
            AnnotationHandlerType::Before | AnnotationHandlerType::After => {
                validate_typed_surface_hook_params(definition, handler)?;
                let minted = mint_hook_body_fn(compiler, definition, handler, index, &config_params);
                install_items.push(BlockItem::Expression(install_call_expr(
                    handler,
                    &minted.name,
                    &config_params,
                )));
                body_fns.push(minted);
            }
        }
    }

    if body_fns.is_empty() {
        return Ok(None);
    }

    // The synthesized `comptime post`-shaped handler. ZERO declared params:
    // the S4b typed injection then delivers every config param as an ordinary
    // TYPED param of the handler mini-program (its value from the
    // `@application` args), exactly as it does for a hand-written zero-param
    // handler — a mismatched application arg is the same loud rejection.
    let post_handler = AnnotationHandler {
        handler_type: AnnotationHandlerType::ComptimePost,
        params: Vec::new(),
        return_type: None,
        body: Expr::Block(
            BlockExpr {
                items: install_items,
            },
            definition.span,
        ),
        span: definition.span,
    };

    Ok(Some(SugarLowering {
        post_handler,
        body_fns,
    }))
}

/// Convenience for the resolution seams: `None` when the definition has no
/// lowering (legacy class, mixed params, no declarative hooks) OR when the
/// lowering rejects — R2/R3 sentences fire ONLY at the declaration through
/// the planner, so the pre-pass ingest never races the attribution.
pub(in crate::compiler) fn sugar_lowering_for_def(
    compiler: &BytecodeCompiler,
    definition: &AnnotationDef,
) -> Option<SugarLowering> {
    match super::planner::classify_annotation_surface(definition) {
        Ok(super::planner::AnnotationSurfaceClass::TypedConfig(_)) => {
            lower_typed_config_declarative_hooks(compiler, definition)
                .ok()
                .flatten()
        }
        _ => None,
    }
}

/// ADR-009 C3 #14 (slice 5, S5b) — the DECLARATION-tier non-function-target
/// rejection (S4 residual 7, dispositioned REJECT): declarative before/after
/// hooks attach to a FUNCTION's call seam; a TypedConfig-with-hooks
/// definition whose EXPLICIT targets exclude `function` could never fire its
/// hooks (the type/module/expression consumer seams run only comptime
/// pre/post handlers — measured silent no-op, S5b probe P-NFT). One
/// producer; the planner fires it after `allowed_targets` is computed.
///
/// The conceivable alternative semantic — "wrap every method of the type" —
/// is NAMED-NOT-BUILT: it would need its own ruling (recorded in the
/// slice-5 report).
pub(in crate::compiler) fn non_function_targets_declaration_rejection(
    definition_name: &str,
    targets: &[AnnotationTargetKind],
) -> String {
    let rendered = targets
        .iter()
        .map(|kind| format!("{kind:?}").to_lowercase())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "annotation `{definition_name}` declares declarative before/after hooks, but its \
         targets ([{rendered}]) do not include function; hook templates attach to a \
         function's call seam and can never fire — add function to targets or remove the \
         hooks"
    )
}

/// ADR-009 C3 #14 (slice 5, S5b) — the APPLICATION-tier non-function-target
/// rejection, fired at the three non-function consumer seams (type:
/// `execute_struct_comptime_handlers`; module:
/// `execute_module_comptime_handlers`; expression:
/// `run_comptime_annotation_handlers_for_target`) when the compiled
/// annotation carries sugar (`sugar_post_handler.is_some()`) — reachable
/// only through a MIXED `targets: [function, …]` definition (a targets-set
/// without `function` already rejected at the declaration).
pub(in crate::compiler) fn non_function_target_application_rejection(
    annotation_name: &str,
    target_kind_word: &str,
    target_name: &str,
) -> String {
    format!(
        "annotation `@{annotation_name}` declares declarative before/after hooks, but is \
         applied to the {target_kind_word} `{target_name}`; hook templates attach to a \
         function's call seam and never fire on {target_kind_word} targets — apply \
         @{annotation_name} to a function, or move the {target_kind_word}-target work into \
         a comptime handler"
    )
}

/// R3: the typed-surface hook-shape rejection (exact charter sentence).
fn validate_typed_surface_hook_params(
    definition: &AnnotationDef,
    handler: &AnnotationHandler,
) -> Result<(), SugarLoweringRejection> {
    let magic_single = handler.params.len() == 1
        && matches!(handler.params[0].name.as_str(), "fn" | "ctx");
    if handler.params.len() > 1 || magic_single || handler.params.iter().any(|p| p.is_variadic) {
        return Err(SugarLoweringRejection {
            message: format!(
                "annotation `{}` declares typed config parameters, which selects the typed \
                 hook surface, but its `{}` handler declares ({}); typed-surface hooks are \
                 before(args) / after(result) / zero-param observers before() / after() — or \
                 remove the parameter types to stay on the legacy surface until it is deleted \
                 (C3-G7/S6)",
                definition.name,
                hook_kind_word(&handler.handler_type),
                render_handler_params(&handler.params),
            ),
            span: handler.span,
        });
    }
    Ok(())
}

fn hook_kind_word(handler_type: &AnnotationHandlerType) -> &'static str {
    match handler_type {
        AnnotationHandlerType::Before => "before",
        AnnotationHandlerType::After => "after",
        AnnotationHandlerType::OnDefine => "on_define",
        AnnotationHandlerType::Metadata => "metadata",
        AnnotationHandlerType::ComptimePre => "comptime pre",
        AnnotationHandlerType::ComptimePost => "comptime post",
    }
}

fn render_handler_params(params: &[AnnotationHandlerParam]) -> String {
    params
        .iter()
        .map(|p| {
            if p.is_variadic {
                format!("...{}", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `(name, declared annotation)` pairs for the definition's config params, in
/// declared order (annotation config params are plain identifiers). The
/// TypedConfig classifier guarantees every annotation is `Some`; a `None`
/// can only mean a mixed definition, which R2-rejects before any lowering.
fn config_param_carrier(definition: &AnnotationDef) -> Vec<(String, TypeAnnotation)> {
    definition
        .params
        .iter()
        .filter_map(|param| {
            let name = param.simple_name()?.to_string();
            let annotation = param.type_annotation.clone()?;
            Some((name, annotation))
        })
        .collect()
}

/// Mint ONE hook body fn (polymorphic or observer form) for a declarative
/// hook. The name + type-param identities are hygienic with a STABLE nonce
/// (definition name + hook kind + handler index) — see the module doc.
fn mint_hook_body_fn(
    compiler: &BytecodeCompiler,
    definition: &AnnotationDef,
    handler: &AnnotationHandler,
    handler_index: usize,
    config_params: &[(String, TypeAnnotation)],
) -> FunctionDef {
    let nonce = stable_hook_nonce(&definition.name, &handler.handler_type, handler_index);
    let fn_name = compiler.mint_hygienic_fn_name_stable(HygienicRole::AnnotationSugarHookBody, nonce);

    let mut params: Vec<FunctionParameter> = Vec::with_capacity(1 + config_params.len());
    let (type_params, return_type) = if let Some(sig_param) = handler.params.first() {
        // Polymorphic form: `fn <minted><Args>(args: Args, <config>) -> Args`
        // (before) / `fn <minted><R>(result: R, <config>) -> R` (after). The
        // pseudo-tuple face keys on the DECLARED param name, so the user's
        // spelling (`args` / `result` / any identifier) addresses it verbatim.
        let type_param_name = compiler
            .mint_hygienic_fn_name_stable(HygienicRole::AnnotationSugarTypeParam, nonce);
        params.push(FunctionParameter {
            pattern: DestructurePattern::Identifier(sig_param.name.clone(), handler.span),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(TypeAnnotation::Basic(type_param_name.clone())),
            default_value: None,
        });
        (
            Some(vec![TypeParam::Type {
                name: type_param_name.clone(),
                span: handler.span,
                doc_comment: None,
                default_type: None,
                trait_bounds: Vec::new(),
            }]),
            Some(TypeAnnotation::Basic(type_param_name)),
        )
    } else {
        // Observer form: `fn <minted>(<config>) { body }` — concrete, zero
        // signature params, no return (F1; target-uniform by design).
        (Some(Vec::new()), None)
    };

    for (name, annotation) in config_params {
        params.push(FunctionParameter {
            pattern: DestructurePattern::Identifier(name.clone(), handler.span),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            // The DECLARED annotation, verbatim (C3-G4/G5).
            type_annotation: Some(annotation.clone()),
            default_value: None,
        });
    }

    FunctionDef {
        name: fn_name,
        name_span: handler.span,
        declaring_module_path: None,
        doc_comment: None,
        type_params,
        params,
        return_type,
        where_clause: None,
        body: hook_body_statements(handler),
        annotations: Vec::new(),
        is_async: false,
        is_comptime: false,
    }
}

fn stable_hook_nonce(
    definition_name: &str,
    handler_type: &AnnotationHandlerType,
    handler_index: usize,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    definition_name.hash(&mut hasher);
    hook_kind_word(handler_type).hash(&mut hasher);
    handler_index.hash(&mut hasher);
    hasher.finish()
}

/// The handler's block body, converted to ordinary fn-body statements — the
/// EXACT desugar of what the same braces would parse to at module scope
/// (`function_body = statement*`), so the minted fn behaves byte-identically
/// to a hand-written body fn. The body is carried VERBATIM: no auto-appended
/// return, no rewrites.
fn hook_body_statements(handler: &AnnotationHandler) -> Vec<Statement> {
    match &handler.body {
        Expr::Block(block, _) => block
            .items
            .iter()
            .map(|item| match item {
                BlockItem::VariableDecl(decl) => {
                    Statement::VariableDecl(decl.clone(), handler.span)
                }
                BlockItem::Assignment(assignment) => {
                    Statement::Assignment(assignment.clone(), handler.span)
                }
                BlockItem::Statement(statement) => statement.clone(),
                BlockItem::Expression(expr) => Statement::Expression(expr.clone(), expr.span()),
            })
            .collect(),
        // The grammar always produces a block; anything else is carried as a
        // single expression statement (defensive, not a rewrite).
        other => vec![Statement::Expression(other.clone(), handler.span)],
    }
}

/// `install(before_hook(<minted_ident>, [capture("p1", p1), …]))` — the ONLY
/// spellings the synthesized handler may emit (the E3 structural whitelist
/// pin asserts exactly this shape).
fn install_call_expr(
    handler: &AnnotationHandler,
    minted_fn_name: &str,
    config_params: &[(String, TypeAnnotation)],
) -> Expr {
    let hook_builtin = match handler.handler_type {
        AnnotationHandlerType::Before => "before_hook",
        _ => "after_hook",
    };
    let captures: Vec<Expr> = config_params
        .iter()
        .map(|(name, _)| Expr::FunctionCall {
            name: "capture".to_string(),
            const_args: Vec::new(),
            args: vec![
                Expr::Literal(Literal::String(name.clone()), handler.span),
                Expr::Identifier(name.clone(), handler.span),
            ],
            named_args: Vec::new(),
            span: handler.span,
        })
        .collect();
    let hook_call = Expr::FunctionCall {
        name: hook_builtin.to_string(),
        const_args: Vec::new(),
        args: vec![
            Expr::Identifier(minted_fn_name.to_string(), handler.span),
            Expr::Array(captures, handler.span),
        ],
        named_args: Vec::new(),
        span: handler.span,
    };
    Expr::FunctionCall {
        name: "install".to_string(),
        const_args: Vec::new(),
        args: vec![hook_call],
        named_args: Vec::new(),
        span: handler.span,
    }
}
