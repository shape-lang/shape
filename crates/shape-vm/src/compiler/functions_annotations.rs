//! Annotation lifecycle and comptime handler compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::{
    Expr, FunctionDef, GeneratedNodeOrigin, Literal, ObjectEntry, Span, Statement, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError, SourceLocation};
use shape_runtime::annotation_context::TargetOwner;
use shape_runtime::comptime_reflection::NominalShape;
use shape_runtime::type_schema::FieldType;
use shape_value::KindedSlot;
use std::collections::{HashMap, HashSet};

use super::comptime_builtins::FrozenTypeIdentity;
use super::comptime_builtins::expansion_provenance::{
    ApplicationClaim, ApplicationId, CanonicalHash, ComptimeStage, DeclarationDiscoveryFixedPoint,
    ExpansionIdentity, ExpansionSite, GeneratedNodePath, GeneratedOrigin, GeneratedSymbolTable,
    GeneratorRef, SourceAnchor, SymbolId, SymbolReservation, TargetIdentity,
};
use super::template_specialization::install_registry::StagedHookInstall;
use super::{BytecodeCompiler, HygienicRole, ParamPassMode};

mod generated_closure_provenance;
use generated_closure_provenance::anchor_generated_function_decl;
mod declaration_discovery;
use declaration_discovery::DeclarationDiscoveryTarget;
// ADR-009 C3 #14 (slice 4): `pub(in crate::compiler)` so the def-param
// carrier producer (`annotation_def_params`) is reachable from the type /
// module / expression annotation-target call sites in `statements.rs` and
// `expressions/mod.rs` — one producer, no per-site re-derivation.
pub(in crate::compiler) mod handler_resolution;
use handler_resolution::{ComptimeAnnotationHandlers, ComptimeHandlerHelperAuthority};
mod original_body_shadow;
use original_body_shadow::{PendingOriginalBodyShadow, canonical_original_callable};

/// ADR-009 C3 #14 (slice 5, S5b): the unspellable mark under which the shared
/// scoped-name collector records a VALUE-position install-family reference
/// (`let f = before_hook`) for the static C3-G8 scan. SOH-prefixed so it can
/// NEVER resolve in any fn table — helper collection sees a lookup miss and
/// skips it, byte-equivalent to the pre-S5b behavior.
const INSTALL_FAMILY_VALUE_MARK: &str = "\u{1}install-family-value:";

#[cfg(test)]
#[path = "functions_annotations/imported_handler_resolution_tests.rs"]
mod imported_handler_resolution_tests;

#[cfg(test)]
#[path = "functions_annotations/handler_helper_authority_tests.rs"]
mod handler_helper_authority_tests;

#[cfg(test)]
#[path = "functions_annotations/c2_slice0_preflight_tests.rs"]
mod c2_slice0_preflight_tests;

#[cfg(test)]
#[path = "functions_annotations/c2_slice2_battery_tests.rs"]
mod c2_slice2_battery_tests;

#[cfg(test)]
#[path = "functions_annotations/c2_slice4_edit_tests.rs"]
mod c2_slice4_edit_tests;

#[cfg(test)]
#[path = "functions_annotations/e2_slice0_spike_tests.rs"]
mod e2_slice0_spike_tests;

#[cfg(test)]
#[path = "functions_annotations/e2_slice3_replace_body_tests.rs"]
mod e2_slice3_replace_body_tests;

#[cfg(test)]
#[path = "functions_annotations/e1_param_selection_tests.rs"]
mod e1_param_selection_tests;

/// ADR-009 E1 #17 (slice 2, E1-D4): the SINGLE spelling->position resolution of
/// a signature directive's parameter against the frozen callable.
///
/// A comptime `set param …` directive names its parameter by SPELLING. E1-D4
/// resolves that spelling to a POSITION exactly once, here, and mints a
/// [`ParamId`]; downstream mutation indexes by the resolved position and never
/// re-resolves the spelling. A spelling the frozen callable does not declare is
/// the named hard error `[C0930]` (`ShapeError::SemanticError`, the slice-1
/// error-class precedent) — never the pre-E1 silent skip that dropped the
/// directive. The `ParamId` field is private to this module, so a position can
/// be obtained ONLY through [`resolve_param_id`].
mod param_selection {
    use shape_ast::ast::FunctionDef;
    use shape_ast::error::{ShapeError, SourceLocation};

    /// A parameter POSITION resolved once against the frozen callable. Minted
    /// only by [`resolve_param_id`] (private field), so the spelling is resolved
    /// at exactly one point.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct ParamId(usize);

    impl ParamId {
        /// The resolved position, for indexing `func_def.params`.
        pub(super) fn index(self) -> usize {
            self.0
        }
    }

    /// Resolve a directive's parameter spelling to a [`ParamId`] against the
    /// frozen callable. A miss is the named hard error `[C0930]` carrying the
    /// directive kind, the missing spelling, and the callable's actual parameter
    /// list — the fail-closed conversion of the pre-E1 silent skip.
    /// `annotation_name`/`location` are the analysis pre-pass's context (the
    /// applying annotation + handler span); the pass-2 install applier has
    /// neither and passes `None`, so the `[C0930]` text drops the `from @…`
    /// clause and carries no span — the same single diagnostic from both call
    /// sites (E1-D4 resolve-ONCE).
    pub(super) fn resolve_param_id(
        func_def: &FunctionDef,
        spelling: &str,
        directive_kind: &str,
        annotation_name: Option<&str>,
        location: Option<&SourceLocation>,
    ) -> Result<ParamId, ShapeError> {
        match func_def
            .params
            .iter()
            .position(|p| p.simple_name() == Some(spelling))
        {
            Some(index) => Ok(ParamId(index)),
            None => {
                let from = annotation_name
                    .map(|name| format!(" from @{name}"))
                    .unwrap_or_default();
                Err(ShapeError::SemanticError {
                    message: format!(
                        "[C0930] comptime `{directive_kind}`{from} on `{}` names parameter \
                         `{spelling}`, which the frozen signature does not declare; its \
                         parameters are [{}]",
                        func_def.name,
                        param_spellings(func_def).join(", ")
                    ),
                    location: location.cloned(),
                })
            }
        }
    }

    /// The frozen callable's declared parameter spellings, in order, for the
    /// `[C0930]` message. A destructuring parameter (no simple name) is shown as
    /// `<destructured>`.
    fn param_spellings(func_def: &FunctionDef) -> Vec<String> {
        func_def
            .params
            .iter()
            .map(|p| {
                p.simple_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "<destructured>".to_string())
            })
            .collect()
    }
}

/// ADR-009 E3 (slice S3, legacy class U11): the TYPED capability a `replace
/// body` replacement reaches through `ctx.original`. It replaces the deleted
/// name-encoded `__original__` alias: the pre-annotation body is compiled into
/// a compiler-issued HYGIENIC shadow function (`shadow_name`, an unspellable
/// [`super::HygienicSymbol`] descriptor — no magic spelling enters the symbol
/// table, rejection-matrix row 2), and `ctx.original` is a typed
/// [`FrozenTypeIdentity`] callable (B6) minted through the single semantic
/// freeze handle (row 3). `ctx.original(args)` in the replacement body is
/// rewritten to a direct typed `FunctionCall` to `shadow_name`
/// (`original_body_rewrite`), so the pre-annotation call is fully typed
/// everywhere downstream.
#[derive(Debug, Clone)]
pub(crate) struct OriginalCapability {
    /// Unspellable registry name of the shadow function holding the
    /// pre-annotation body.
    shadow_name: String,
    /// The shadow's frozen callable signature identity — the typed
    /// `FrozenCallable` (B6) `ctx.original` denotes.
    callable: FrozenTypeIdentity,
}

impl OriginalCapability {
    fn shadow_name(&self) -> &str {
        &self.shadow_name
    }

    fn callable(&self) -> FrozenTypeIdentity {
        self.callable
    }
}

/// Canonical label for the comptime handler kind inside a generator
/// descriptor. Total over `AnnotationHandlerType` — only the two comptime
/// kinds reach the expansion path today, but the descriptor never fabricates.
fn annotation_handler_kind_descriptor(
    handler_type: &shape_ast::ast::AnnotationHandlerType,
) -> &'static str {
    use shape_ast::ast::AnnotationHandlerType;
    match handler_type {
        AnnotationHandlerType::ComptimePre => "comptime-pre",
        AnnotationHandlerType::ComptimePost => "comptime-post",
        AnnotationHandlerType::OnDefine => "on-define",
        AnnotationHandlerType::Before => "before",
        AnnotationHandlerType::After => "after",
        AnnotationHandlerType::Metadata => "metadata",
    }
}

/// Canonical label for the annotated target's kind inside a target-identity
/// descriptor.
fn annotation_target_kind_descriptor(
    kind: shape_ast::ast::functions::AnnotationTargetKind,
) -> &'static str {
    use shape_ast::ast::functions::AnnotationTargetKind;
    match kind {
        AnnotationTargetKind::Function => "function",
        AnnotationTargetKind::Type => "type",
        AnnotationTargetKind::Module => "module",
        AnnotationTargetKind::Expression => "expression",
        AnnotationTargetKind::Block => "block",
        AnnotationTargetKind::AwaitExpr => "await-expr",
        AnnotationTargetKind::Binding => "binding",
    }
}

/// Canonical dependency descriptors of one comptime expansion for ticket D1:
/// exactly what the existing path FEEDS the handler — the `ComptimeTarget`
/// the handler receives (fields with their type strings and field
/// annotations, params, return type, applied annotations, captures). The
/// full declaration-discovery dependency graph is ticket D2.
fn comptime_target_dependency_descriptors(
    target: &super::comptime_target::ComptimeTarget,
) -> Vec<String> {
    let mut descriptors = Vec::new();
    for (field_name, field_type, field_annotations) in &target.fields {
        descriptors.push(format!("field:{field_name}:{field_type}"));
        for (ann_name, ann_args) in field_annotations {
            descriptors.push(format!(
                "field-annotation:{field_name}:{ann_name}:{}",
                ann_args.join(",")
            ));
        }
    }
    for (param_name, param_type, is_const) in &target.params {
        descriptors.push(format!("param:{param_name}:{param_type}:{is_const}"));
    }
    if let Some(return_type) = &target.return_type {
        descriptors.push(format!("return:{return_type}"));
    }
    for applied in &target.annotations {
        descriptors.push(format!("applied-annotation:{applied}"));
    }
    for capture in &target.captures {
        descriptors.push(format!("capture-name:{capture}"));
    }
    descriptors
}

/// Canonical structural content encoding of a generated `extend` method
/// (rejection row 3's conflicting-output detector). Taken over the
/// handler-emitted AST — post target-substitution, PRE parameter-annotation
/// enrichment — so the speculative pre-pass and the authoritative pass-2
/// run of one application encode equal output equally.
fn generated_extend_method_content(
    type_name: &shape_ast::ast::TypeName,
    method: &shape_ast::ast::types::MethodDef,
) -> CanonicalHash {
    CanonicalHash::from_canonical_decl_encoding(&format!("extend:{type_name:?}:{method:?}"))
}

/// Canonical structural content encoding of a generated free function.
/// (`pub(in crate::compiler)`: the S2c hook-template weave shadow
/// reservation in `template_specialization/weave.rs` encodes its shadow def
/// through the SAME encoder — visibility-only widening, byte-identical
/// behavior.)
pub(in crate::compiler) fn generated_free_fn_content(func_def: &FunctionDef) -> CanonicalHash {
    CanonicalHash::from_canonical_decl_encoding(&format!("fn:{func_def:?}"))
}

impl BytecodeCompiler {
    pub(super) fn apply_function_comptime_signature_directives_for_analysis(
        &mut self,
        program: &mut shape_ast::ast::Program,
    ) -> Result<()> {
        let handler_map = self.collect_comptime_annotation_handlers(program)?;
        if handler_map.is_empty() {
            return Ok(());
        }

        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let ctx_module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        // ADR-009 C3 #14 (slice 5, S5b): the syntactic fn table for the
        // static C3-G8 scan — collected ONCE per pre-pass run, from the same
        // analysis items the handler_map ingested (scan-only; see
        // `collect_pre_pass_ast_function_defs`).
        let mut ast_fn_defs = HashMap::new();
        Self::collect_pre_pass_ast_function_defs(&program.items, None, &mut ast_fn_defs);

        Self::apply_function_comptime_signature_directives_to_items(
            self,
            &handler_map,
            &extensions,
            &trait_impls,
            &known_type_symbols,
            &ctx_module_path,
            &ctx_file,
            &ast_fn_defs,
            &mut program.items,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_function_comptime_signature_directives_to_items(
        compiler: &mut BytecodeCompiler,
        handler_map: &HashMap<String, ComptimeAnnotationHandlers>,
        extensions: &[shape_runtime::module_exports::ModuleExports],
        trait_impls: &std::collections::HashSet<String>,
        known_type_symbols: &HashSet<String>,
        ctx_module_path: &str,
        ctx_file: &str,
        ast_fn_defs: &HashMap<String, FunctionDef>,
        items: &mut [shape_ast::ast::Item],
    ) -> Result<()> {
        use shape_ast::ast::{ExportItem, Item};

        for item in items {
            match item {
                Item::Function(func, _) => {
                    compiler.apply_function_comptime_signature_directives_to_function(
                        handler_map,
                        extensions,
                        trait_impls,
                        known_type_symbols,
                        ctx_module_path,
                        ctx_file,
                        ast_fn_defs,
                        func,
                    )?;
                }
                Item::Export(export, _) => {
                    if let ExportItem::Function(func) = &mut export.item {
                        compiler.apply_function_comptime_signature_directives_to_function(
                            handler_map,
                            extensions,
                            trait_impls,
                            known_type_symbols,
                            ctx_module_path,
                            ctx_file,
                            ast_fn_defs,
                            func,
                        )?;
                    }
                }
                Item::Module(module, _) => {
                    let module_path = if ctx_module_path.is_empty() {
                        module.name.clone()
                    } else {
                        Self::qualify_module_symbol(ctx_module_path, &module.name)
                    };
                    compiler.with_comptime_annotation_module_scope(
                        module_path.clone(),
                        |compiler| {
                            Self::apply_function_comptime_signature_directives_to_items(
                                compiler,
                                handler_map,
                                extensions,
                                trait_impls,
                                known_type_symbols,
                                &module_path,
                                ctx_file,
                                ast_fn_defs,
                                &mut module.items,
                            )
                        },
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// ADR-009 C3 #14 (slice 5, S5a) — THE ONE [C0931] producer: the Dec-65
    /// config-arg pre-check. A `@application` config argument whose free
    /// identifier resolves (scoped-then-bare, the same detector as the
    /// [C0926] gate) to a NON-const module binding is rejected BEFORE the
    /// handler mini-VM runs — so the pre-pass error-swallow (the
    /// `[comptime error]`-filtered `continue` below each execute seam)
    /// cannot eat it, and the bland mini-VM `[C0001] Undefined variable`
    /// failure (S5a probes P3a/P3b, recorded in c3-slice5-report.md)
    /// upgrades to a named sentence. Diagnostic upgrade ONLY: every shape
    /// this rejects failed loudly before (the mini-VM cannot see module
    /// bindings at all); nothing that ran keeps running differently.
    ///
    /// THE ASYMMETRY RULE (invariant 7): a module-scope CONST is
    /// comptime-evaluable and therefore LEGAL in the config-arg position —
    /// `const_module_bindings` members, injected specialization
    /// `const_bindings`, and imported `pub const`s are all EXEMPT — while
    /// the SAME const is ILLEGAL inside a template body ([C0926]: G4
    /// exact-inputs totality covers consts; the positive twin is "declare
    /// it as a capture"). NOTE (measured, S5a probe P6): a top-level
    /// `const` config arg is exempt here but STILL fails loudly today with
    /// the pre-existing `[C0001] Undefined variable` — the mini-VM has no
    /// const-injection route yet; making module consts VISIBLE in the
    /// config position is a named follow-up, not this check's charter.
    ///
    /// Fires for BOTH surface classes (TypedConfig and Legacy definitions
    /// with comptime handlers — the class-independent comptime seams are
    /// the only routes by which config enters a comptime evaluation
    /// position; legacy RUNTIME-hook config stays per-invocation and
    /// untouched until S6).
    fn reject_runtime_module_binding_config_args(
        &self,
        ann: &shape_ast::ast::Annotation,
        const_bindings: &[(String, KindedSlot)],
    ) -> Result<()> {
        for arg in &ann.args {
            let mut names: Vec<String> = Vec::new();
            Self::collect_config_arg_value_names(arg, &mut names);
            for ident in names {
                if const_bindings.iter().any(|(name, _)| name == &ident) {
                    continue;
                }
                if self.imported_consts.contains_key(&ident) {
                    continue;
                }
                let Some(resolved) = self.resolve_scoped_module_binding_name(&ident) else {
                    continue;
                };
                let Some(&binding_idx) = self.module_bindings.get(&resolved) else {
                    continue;
                };
                if self.const_module_bindings.contains(&binding_idx) {
                    continue;
                }
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[C0931] config argument `{ident}` for `@{}` references a runtime \
                         module binding; annotation config is evaluated once at compile time \
                         (Dec 65 — runtime values never enter a comptime evaluation position) \
                         — pass a literal or a comptime const; a value that varies at runtime \
                         cannot configure a compile-time specialization",
                        ann.name
                    ),
                    location: Some(self.span_to_source_location(ann.span)),
                });
            }
        }
        Ok(())
    }

    /// The [C0931] detector's conservative free-identifier collector over a
    /// config-arg expression: value-position identifiers only (call NAMES
    /// are skipped — fn callees are comptime helpers, not values), plus
    /// f-string interpolation interiors (re-parsed exactly as the emitter
    /// does). Unrecognized expression shapes are NOT recursed — a missed
    /// name falls through to the mini-VM's pre-existing loud unresolved
    /// error, never a silent pass.
    fn collect_config_arg_value_names(expr: &Expr, names: &mut Vec<String>) {
        match expr {
            Expr::Identifier(name, _) => names.push(name.clone()),
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::collect_config_arg_value_names(left, names);
                Self::collect_config_arg_value_names(right, names);
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::Reference { expr: operand, .. } => {
                Self::collect_config_arg_value_names(operand, names);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_config_arg_value_names(condition, names);
                Self::collect_config_arg_value_names(then_expr, names);
                if let Some(else_expr) = else_expr {
                    Self::collect_config_arg_value_names(else_expr, names);
                }
            }
            Expr::Array(items, _) => {
                for item in items {
                    Self::collect_config_arg_value_names(item, names);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } | ObjectEntry::Spread(value) => {
                            Self::collect_config_arg_value_names(value, names);
                        }
                    }
                }
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::collect_config_arg_value_names(object, names);
                Self::collect_config_arg_value_names(index, names);
                if let Some(end) = end_index {
                    Self::collect_config_arg_value_names(end, names);
                }
            }
            Expr::PropertyAccess { object, .. } => {
                Self::collect_config_arg_value_names(object, names);
            }
            Expr::MethodCall { receiver, args, named_args, .. } => {
                Self::collect_config_arg_value_names(receiver, names);
                for arg in args {
                    Self::collect_config_arg_value_names(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_config_arg_value_names(value, names);
                }
            }
            Expr::FunctionCall { const_args, args, named_args, .. }
            | Expr::QualifiedFunctionCall { const_args, args, named_args, .. } => {
                for arg in const_args {
                    Self::collect_config_arg_value_names(arg, names);
                }
                for arg in args {
                    Self::collect_config_arg_value_names(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_config_arg_value_names(value, names);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    Self::collect_config_arg_value_names(start, names);
                }
                if let Some(end) = end {
                    Self::collect_config_arg_value_names(end, names);
                }
            }
            Expr::Literal(Literal::FormattedString { value, mode }, _) => {
                let Ok(parts) =
                    shape_ast::interpolation::parse_interpolation_with_mode(value, *mode)
                else {
                    return;
                };
                for part in parts {
                    if let shape_ast::interpolation::InterpolationPart::Expression {
                        expr, ..
                    } = part
                    {
                        if let Ok(parsed) = shape_ast::parser::parse_expression_str(&expr) {
                            Self::collect_config_arg_value_names(&parsed, names);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// ADR-009 C3 #14 (slice 5, S5b) — THE STATIC C3-G8 ARM.
    ///
    /// The measured hole (S5b probe P-G8b, recorded in c3-slice5-report.md):
    /// an API-path installing annotation on an UNCALLED generic target in a
    /// single-module unit was a SILENT NO-OP — the pre-pass handler run fails
    /// (body-fn lookup precedes function registration there), the
    /// `[comptime error]`-filtered swallow defers to pass-2, and pass-2 skips
    /// a generic def's body compile entirely, so no consumer ever observed
    /// the install. This check is STATIC: it keys on the target's
    /// `type_params` and a SYNTACTIC template-engagement classification of
    /// the resolved annotation entry — NO handler-run dependence — and fires
    /// at the `@application` span through the EXISTING ONE C3-G8 sentence
    /// producer (`generic_target_install_rejection_message`, wording
    /// byte-unchanged). The three dynamic firing sites (the pre-pass
    /// directive arm + the two `apply_install_hook_template` twins) REMAIN
    /// as layered backstops.
    ///
    /// Concrete targets are untouched (the `type_params` key). LEGACY-weave
    /// annotations (declarative hooks, no typed config, no install-family
    /// references in their comptime handlers) are NOT template-engaging —
    /// the S0 g1/g2/g4/g5 accidental-working class keeps working until S6
    /// (deliberate; the C3-G11 defections.md entry covers it).
    fn reject_template_engaging_annotation_on_generic_target(
        &self,
        ann: &shape_ast::ast::Annotation,
        entry: &ComptimeAnnotationHandlers,
        func_def: &FunctionDef,
        ctx_module_path: &str,
        ast_fn_defs: &HashMap<String, FunctionDef>,
    ) -> Result<()> {
        if !func_def
            .type_params
            .as_ref()
            .is_some_and(|params| !params.is_empty())
        {
            return Ok(());
        }
        let Some(body_fn_hint) =
            self.template_engaging_install_reference(entry, ctx_module_path, ast_fn_defs)
        else {
            return Ok(());
        };
        Err(ShapeError::SemanticError {
            message: super::template_specialization::install_registry::
                generic_target_install_rejection_message(&body_fn_hint, &ann.name, func_def),
            location: Some(self.span_to_source_location(ann.span)),
        })
    }

    /// The STATIC template-engagement classification (conservative,
    /// syntactic — the Lens-2 F4 conservative-set precedent: an
    /// over-approximation is only ever the LOUD C3-G8 rejection, never a
    /// silent accept). An entry is template-engaging iff:
    ///
    /// (a) sugar path — its `sugar_body_fns` is non-empty (a
    ///     TypedConfig-with-hooks definition's synthesized install handler
    ///     exists by construction); the hint is the first minted body fn
    ///     (the same name the dynamic directive arm renders); OR
    /// (b) API path — a syntactic scan of its comptime handler bodies AND
    ///     their transitive comptime helpers finds the `install` name in
    ///     CALL-NAME or VALUE position — value position included so
    ///     `let f = install; f(t)` cannot dodge the scan (S5b probe P-G8d
    ///     measured the value-position dodge silent on a generic target).
    ///     ENGAGEMENT keys on `install` ONLY (the sole installer — the
    ///     other family names `before_hook`/`after_hook`/`*_nocapture`
    ///     CONSTRUCT template handles and cannot install anything): C3-G8
    ///     withdraws INSTALLS on generic targets, and a construct-only
    ///     handler on a generic target is legal load-bearing machinery
    ///     (the fix-round-1 F5 store-lifecycle refuter annotates a
    ///     polymorphic template BODY fn with a handler that pushes two
    ///     templates and installs nothing — measured-green baseline pin
    ///     `nested_handler_run_during_processing_does_not_shift_install_
    ///     handles`; a five-name key rejected it, disclosed in the slice-5
    ///     report). The constructor names still feed the body-fn HINT.
    ///
    /// Helper transitivity: `collect_authorized_comptime_helpers` IS
    /// worklist-transitive over `self.function_defs`, and is reused verbatim
    /// — but in a single-module unit BOTH pre-passes run before function
    /// registration (the S2b measured reach), so the registered table is
    /// empty exactly where the uncalled-generic hole lives. The scan
    /// therefore ALSO closes over `ast_fn_defs`, the SYNTACTIC fn table
    /// collected from the analysis program's own items (bare +
    /// module-qualified names) — a disclosed S5b addition, scan-only (never
    /// an execution surface).
    ///
    /// The hint is the first hook-constructor's first argument when it is a
    /// bare identifier (`before_hook(my_before, …)` → `my_before` — the
    /// name the dynamic pass-2 seam would have rendered); otherwise the
    /// established `"<template>"` placeholder (the directive-arm precedent).
    /// Best-effort ONLY for the hint; engagement never depends on it.
    fn template_engaging_install_reference(
        &self,
        entry: &ComptimeAnnotationHandlers,
        ctx_module_path: &str,
        ast_fn_defs: &HashMap<String, FunctionDef>,
    ) -> Option<String> {
        if let Some(first) = entry.sugar_body_fns.first() {
            return Some(first.name.clone());
        }
        let handler_module_path = entry
            .defining_module_path
            .as_deref()
            .unwrap_or(ctx_module_path);
        let mut engaged = false;
        let mut hint: Option<String> = None;
        let mut pending: Vec<String> = Vec::new();
        let absorb = |names: HashSet<String>, pending: &mut Vec<String>, engaged: &mut bool| {
            for name in names {
                if Self::scoped_name_engages_install(&name) {
                    *engaged = true;
                }
                pending.push(name);
            }
        };
        for handler in &entry.handlers {
            let mut seeds = HashSet::new();
            Self::collect_scoped_names_in_expr(&handler.body, &mut seeds);
            absorb(seeds, &mut pending, &mut engaged);
            if hint.is_none() {
                hint = Self::hook_constructor_hint_in_expr(&handler.body);
            }
            // (reused, transitive) — the authoritative registered-table
            // closure, exactly what handler execution would authorize.
            for helper in
                self.collect_authorized_comptime_helpers(&handler.body, entry.helper_authority())
            {
                for statement in &helper.body {
                    let mut nested = HashSet::new();
                    Self::collect_scoped_names_in_statement(statement, &mut nested);
                    absorb(nested, &mut pending, &mut engaged);
                    if hint.is_none() {
                        hint = Self::hook_constructor_hint_in_statement(statement);
                    }
                }
            }
        }
        // The AST-side syntactic closure (pre-registration complement).
        let mut visited: HashSet<String> = HashSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let definition = ast_fn_defs.get(&name).or_else(|| {
                ast_fn_defs.get(&Self::qualify_module_symbol(handler_module_path, &name))
            });
            let Some(definition) = definition else {
                continue;
            };
            for statement in &definition.body {
                let mut nested = HashSet::new();
                Self::collect_scoped_names_in_statement(statement, &mut nested);
                absorb(nested, &mut pending, &mut engaged);
                if hint.is_none() {
                    hint = Self::hook_constructor_hint_in_statement(statement);
                }
            }
        }
        engaged.then(|| hint.unwrap_or_else(|| "<template>".to_string()))
    }

    /// The ENGAGEMENT test over ONE collected scoped name (S5b static
    /// C3-G8): the `install` name — the sole installer — matched bare, as
    /// the last `::` segment of a qualified call name, or under the
    /// [`INSTALL_FAMILY_VALUE_MARK`] the shared collector stamps on
    /// VALUE-position references. Within the install key,
    /// over-approximation (e.g. a user fn spelled `install` referenced from
    /// a handler) is only ever the LOUD C3-G8 rejection (the Lens-2 F4
    /// conservative-set precedent); the constructor family names do NOT
    /// engage (see `template_engaging_install_reference`).
    fn scoped_name_engages_install(name: &str) -> bool {
        if let Some(marked) = name.strip_prefix(INSTALL_FAMILY_VALUE_MARK) {
            return marked == "install";
        }
        let last = name.rsplit("::").next().unwrap_or(name);
        last == "install"
    }

    /// The five install-family spellings (`comptime.rs` forwarder rows; the
    /// SOH-prefixed nocapture forwarders are unspellable, so their PLAIN
    /// builtin names are the reachable surface). Used by the shared
    /// collector's VALUE-position mark arm; ENGAGEMENT itself keys on
    /// `install` only (`scoped_name_engages_install`).
    fn is_install_family_name(name: &str) -> bool {
        matches!(
            name,
            "install"
                | "before_hook"
                | "after_hook"
                | "before_hook_nocapture"
                | "after_hook_nocapture"
        )
    }

    /// Best-effort body-fn HINT for the static C3-G8 sentence: the first
    /// hook-constructor call whose first argument is a bare identifier
    /// (`before_hook(my_before, …)` → `my_before`). Covers the realistic
    /// handler spellings (block/expression statements, let-initializers,
    /// nested call arguments, if/else branches); anything more exotic falls
    /// back to `"<template>"` at the caller — the hint can be less specific,
    /// never wrong, and ENGAGEMENT never depends on it.
    fn hook_constructor_hint_in_expr(expr: &Expr) -> Option<String> {
        match expr {
            Expr::FunctionCall { name, args, .. } => {
                if matches!(name.as_str(), "before_hook" | "after_hook") {
                    if let Some(Expr::Identifier(body_fn, _)) = args.first() {
                        return Some(body_fn.clone());
                    }
                }
                args.iter().find_map(Self::hook_constructor_hint_in_expr)
            }
            Expr::QualifiedFunctionCall { function, args, .. } => {
                if matches!(function.as_str(), "before_hook" | "after_hook") {
                    if let Some(Expr::Identifier(body_fn, _)) = args.first() {
                        return Some(body_fn.clone());
                    }
                }
                args.iter().find_map(Self::hook_constructor_hint_in_expr)
            }
            Expr::Block(block, _) => block.items.iter().find_map(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .and_then(Self::hook_constructor_hint_in_expr),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    Self::hook_constructor_hint_in_expr(&assign.value)
                }
                shape_ast::ast::BlockItem::Statement(statement) => {
                    Self::hook_constructor_hint_in_statement(statement)
                }
                shape_ast::ast::BlockItem::Expression(expr) => {
                    Self::hook_constructor_hint_in_expr(expr)
                }
            }),
            Expr::If(if_expr, _) => Self::hook_constructor_hint_in_expr(&if_expr.then_branch)
                .or_else(|| {
                    if_expr
                        .else_branch
                        .as_deref()
                        .and_then(Self::hook_constructor_hint_in_expr)
                }),
            _ => None,
        }
    }

    fn hook_constructor_hint_in_statement(statement: &Statement) -> Option<String> {
        match statement {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                Self::hook_constructor_hint_in_expr(expr)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .and_then(Self::hook_constructor_hint_in_expr),
            Statement::Assignment(assign, _) => {
                Self::hook_constructor_hint_in_expr(&assign.value)
            }
            Statement::If(if_stmt, _) => if_stmt
                .then_body
                .iter()
                .find_map(Self::hook_constructor_hint_in_statement)
                .or_else(|| {
                    if_stmt.else_body.as_ref().and_then(|body| {
                        body.iter()
                            .find_map(Self::hook_constructor_hint_in_statement)
                    })
                }),
            _ => None,
        }
    }

    /// Collect the analysis program's fn definitions into a SYNTACTIC name
    /// table for the static C3-G8 scan (S5b): top-level + `export` fns under
    /// their bare names, module fns under their qualified names, recursively.
    /// Scan-only — never consulted for execution or registration.
    fn collect_pre_pass_ast_function_defs(
        items: &[shape_ast::ast::Item],
        module_path: Option<&str>,
        table: &mut HashMap<String, FunctionDef>,
    ) {
        use shape_ast::ast::{ExportItem, Item};
        for item in items {
            match item {
                Item::Function(func, _) => {
                    let name = match module_path {
                        Some(module) => Self::qualify_module_symbol(module, &func.name),
                        None => func.name.clone(),
                    };
                    table.entry(name).or_insert_with(|| func.clone());
                }
                Item::Export(export, _) => {
                    if let ExportItem::Function(func) = &export.item {
                        let name = match module_path {
                            Some(module) => Self::qualify_module_symbol(module, &func.name),
                            None => func.name.clone(),
                        };
                        table.entry(name).or_insert_with(|| func.clone());
                    }
                }
                Item::Module(module, _) => {
                    let nested = match module_path {
                        Some(parent) => Self::qualify_module_symbol(parent, &module.name),
                        None => module.name.clone(),
                    };
                    Self::collect_pre_pass_ast_function_defs(
                        &module.items,
                        Some(&nested),
                        table,
                    );
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_function_comptime_signature_directives_to_function(
        &mut self,
        handler_map: &HashMap<String, ComptimeAnnotationHandlers>,
        extensions: &[shape_runtime::module_exports::ModuleExports],
        trait_impls: &std::collections::HashSet<String>,
        known_type_symbols: &HashSet<String>,
        ctx_module_path: &str,
        ctx_file: &str,
        ast_fn_defs: &HashMap<String, FunctionDef>,
        func_def: &mut FunctionDef,
    ) -> Result<()> {
        use shape_ast::ast::AnnotationHandlerType;

        let annotations = func_def.annotations.clone();
        // ADR-009 C3 #14 (slice 5, S5b): the static C3-G8 arm fires FIRST —
        // per `@application`, before any handler execution, with no
        // handler-run dependence (see the arm's doc). Resolution here is a
        // pure map lookup, never execution.
        for ann in &annotations {
            let Some((_, entry)) = self.resolve_comptime_annotation_handlers(
                handler_map,
                ann,
                (!ctx_module_path.is_empty()).then_some(ctx_module_path),
            ) else {
                continue;
            };
            self.reject_template_engaging_annotation_on_generic_target(
                ann,
                entry,
                func_def,
                ctx_module_path,
                ast_fn_defs,
            )?;
        }
        let phases = [
            AnnotationHandlerType::ComptimePre,
            AnnotationHandlerType::ComptimePost,
        ];
        for phase in phases.iter() {
            for ann in &annotations {
                let Some((_, entry)) = self.resolve_comptime_annotation_handlers(
                    &handler_map,
                    ann,
                    (!ctx_module_path.is_empty()).then_some(ctx_module_path),
                ) else {
                    continue;
                };
                for handler in entry.handlers.iter().filter(|h| &h.handler_type == phase) {
                    let target = super::comptime_target::ComptimeTarget::from_function(func_def);
                    // S3 pre-pass freeze rule (see `s3_freeze_gate_tests`
                    // module doc): this signature-directive pre-pass runs
                    // AFTER the semantic-freeze barrier and consumes the
                    // real registration-complete freeze handle — the same
                    // one pass-2 uses. A site that cannot obtain it is the
                    // row-3 named compile error; the handle is acquired
                    // before the output-suppression toggle so the error
                    // path cannot leak suppression state.
                    //
                    // ADR-009 E1 #17 (slice 5): acquired BEFORE `to_nanboxed` so
                    // the SAME `Arc<FreezeOverlay>` both stamps the target's
                    // `type_ref` identities (producer stamp-gate) AND is threaded
                    // to the handler executor below — a composite identity
                    // interned at stamp time lives in this overlay's shared memo
                    // and is visible to the consumer's `payload_of`.
                    let freeze = self.comptime_freeze_overlay()?;
                    let target_value = target.to_nanboxed(Some(freeze.as_ref()))?;
                    let handler_module_path = entry
                        .defining_module_path
                        .as_deref()
                        .unwrap_or(ctx_module_path);
                    let helpers = self.collect_authorized_comptime_helpers(
                        &handler.body,
                        entry.helper_authority(),
                    );

                    // ADR-009 C3 #14 (slice 2): the hook-template body-fn
                    // lookup (same AST fn table as the helper collection;
                    // threaded as a parameter). A root fn not yet registered
                    // at this pre-pass simply misses here and the named
                    // rejection defers this handler to pass-2 (the
                    // established pre-pass fallback below).
                    //
                    // S4c: the entry's MINTED sugar body fns resolve FIRST
                    // (hygienic names — no user fn can collide).
                    let function_defs = &self.function_defs;
                    let sugar_body_fns = &entry.sugar_body_fns;
                    let template_body_fn_lookup = move |name: &str| -> Option<FunctionDef> {
                        sugar_body_fns
                            .iter()
                            .find(|def| def.name == name)
                            .cloned()
                            .or_else(|| function_defs.get(name).cloned())
                            .or_else(|| {
                                function_defs
                                    .get(&Self::qualify_module_symbol(handler_module_path, name))
                                    .cloned()
                            })
                    };
                    // ADR-009 C3 #14 (slice 5, S5a) — the [C0931] Dec-65
                    // config-arg pre-check: returns Err BEFORE execution so
                    // the `[comptime error]`-filtered swallow below cannot
                    // eat it.
                    self.reject_runtime_module_binding_config_args(ann, &[])?;
                    let prev_suppressed =
                        super::comptime_builtins::set_comptime_output_suppressed(true);
                    let execution_result =
                        super::comptime::execute_comptime_with_annotation_handler(
                            &handler.body,
                            &handler.params,
                            target_value,
                            &ann.args,
                            &entry.def_params,
                            &[],
                            &helpers,
                            extensions,
                            known_type_symbols.clone(),
                            handler_module_path,
                            ctx_file,
                            trait_impls.clone(),
                            freeze,
                            // Function-target handler: no representation authority.
                            None,
                            &template_body_fn_lookup,
                        );
                    super::comptime_builtins::set_comptime_output_suppressed(prev_suppressed);

                    let execution = match execution_result {
                        Ok(execution) => execution,
                        Err(e) => {
                            if e.to_string().contains("[comptime error]") {
                                let context =
                                    format!("the @{} annotation on {}", ann.name, func_def.name);
                                return Err(self.build_comptime_failure(&e, ann.span, &context));
                            }
                            continue;
                        }
                    };
                    let handler_location = self.span_to_source_location(handler.span);
                    // ADR-009 C3 #14 (slice 2, S2b): the `@application`
                    // anchor for the C3-G8 generic-target install rejection
                    // (the only consumer observing the generic def's real
                    // `type_params` — see the InstallHookTemplate arm).
                    let application_location = self.span_to_source_location(ann.span);
                    Self::apply_signature_directives_to_analysis_function(
                        func_def,
                        execution.directives,
                        &ann.name,
                        handler_location,
                        application_location,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Apply a comptime handler's signature directives to the analysis
    /// `FunctionDef`. ADR-009 E1-D4 (slice 2): each directive's parameter
    /// SPELLING is resolved to a POSITION exactly once, via
    /// [`param_selection::resolve_param_id`], and the resolved [`ParamId`] is
    /// what the mutation indexes — the spelling is never re-resolved. A spelling
    /// the frozen callable does not declare is the named hard error `[C0930]`
    /// (never the pre-E1 silent skip that dropped the directive).
    fn apply_signature_directives_to_analysis_function(
        func_def: &mut FunctionDef,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        annotation_name: &str,
        handler_location: SourceLocation,
        // ADR-009 C3 #14 (slice 2, S2b): the `@application` anchor for the
        // C3-G8 generic-target install rejection (see that arm).
        application_location: SourceLocation,
    ) -> std::result::Result<(), ShapeError> {
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::SetParamType {
                    param_name,
                    type_annotation,
                } => {
                    let param_id = param_selection::resolve_param_id(
                        func_def,
                        &param_name,
                        "set param type",
                        Some(annotation_name),
                        Some(&handler_location),
                    )?;
                    let param = &mut func_def.params[param_id.index()];
                    if let Some(existing) = &param.type_annotation {
                        if existing != &type_annotation {
                            return Err(ShapeError::RuntimeError {
                                message: format!(
                                    "Comptime handler '{}' directive processing failed: cannot \
                                     override explicit type of parameter '{}'",
                                    annotation_name, param_name
                                ),
                                location: Some(handler_location.clone()),
                            });
                        }
                    } else {
                        param.type_annotation = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::SetParamValue {
                    param_name,
                    value,
                } => {
                    let param_id = param_selection::resolve_param_id(
                        func_def,
                        &param_name,
                        "set param value",
                        Some(annotation_name),
                        Some(&handler_location),
                    )?;
                    let default_value =
                        Self::scalar_default_expr_from_kinded_slot(&param_name, &value).map_err(
                            |message| ShapeError::RuntimeError {
                                message: format!(
                                    "Comptime handler '{}' directive processing failed: {}",
                                    annotation_name, message
                                ),
                                location: Some(handler_location.clone()),
                            },
                        )?;
                    func_def.params[param_id.index()].default_value = Some(default_value);
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {}
                // ADR-009 C3 #14 (slice 2, S2b): documented PRE-PASS no-op
                // for the APPLY — install applies at the authoritative pass-2
                // function-target consumer
                // (`process_comptime_directives_for_function`) ONLY, never
                // here; a pre-pass apply would double-install (one registry
                // row / staged install per application, across the pre-pass +
                // pass-2 double handler run).
                //
                // The ONE exception is the C3-G8 GENERIC-TARGET rejection,
                // which fires HERE (disclosed narrowing of the "documented
                // no-op" plan resolution): a generic def's pass-2 body
                // compile is skipped entirely (`functions.rs`
                // compile_function_with_generated_origin — "Skip compiling
                // bodies of generic extend methods"), and a monomorphized
                // specialization reaches pass-2 with `type_params` already
                // cleared — so this pre-pass consumer is the ONLY directive
                // consumer that observes the generic def's real
                // `type_params`. A rejection is surface-and-stop, not an
                // apply: nothing installs, nothing doubles. The pass-2 seam
                // keeps two defensive twins of the same sentence
                // (`apply_install_hook_template`).
                super::comptime_builtins::ComptimeDirective::InstallHookTemplate {
                    template_index,
                } => {
                    if func_def
                        .type_params
                        .as_ref()
                        .is_some_and(|params| !params.is_empty())
                    {
                        let template_body_fn =
                            super::comptime_builtins::comptime_hook_template_at(template_index)
                                .map(|bound| bound.template.body_fn().to_string())
                                .unwrap_or_else(|| "<template>".to_string());
                        return Err(ShapeError::SemanticError {
                            message: super::template_specialization::install_registry::
                                generic_target_install_rejection_message(
                                    &template_body_fn,
                                    annotation_name,
                                    func_def,
                                ),
                            location: Some(application_location.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn scalar_default_expr_from_kinded_slot(
        param_name: &str,
        value: &KindedSlot,
    ) -> std::result::Result<Expr, String> {
        let coerce_to_f64 = |slot: &KindedSlot| -> Option<f64> {
            match slot.kind() {
                shape_value::NativeKind::Int64 => slot.as_i64().map(|i| i as f64),
                shape_value::NativeKind::Float64 => slot.as_f64(),
                _ => None,
            }
        };
        if let Some(i) = value.as_i64() {
            Ok(Expr::Literal(Literal::Int(i), Span::DUMMY))
        } else if let Some(n) = coerce_to_f64(value) {
            Ok(Expr::Literal(Literal::Number(n), Span::DUMMY))
        } else if let Some(b) = value.as_bool() {
            Ok(Expr::Literal(Literal::Bool(b), Span::DUMMY))
        } else if let Some(s) = value.as_str() {
            Ok(Expr::Literal(Literal::String(s.to_string()), Span::DUMMY))
        } else if matches!(value.kind(), shape_value::NativeKind::Null) {
            Ok(Expr::Literal(Literal::None, Span::DUMMY))
        } else {
            Err(format!(
                "unsupported default value for parameter '{}': set param value only supports int, number, bool, string, and none scalars in this lane (got {:?})",
                param_name,
                value.kind()
            ))
        }
    }

    fn annotation_type_is_unknown(annotation: &TypeAnnotation) -> bool {
        match annotation {
            TypeAnnotation::Basic(name) => name == "unknown",
            TypeAnnotation::Reference(path) => path.as_str() == "unknown",
            TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
                Self::annotation_type_is_unknown(inner)
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => {
                items.iter().any(Self::annotation_type_is_unknown)
            }
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| Self::annotation_type_is_unknown(&field.type_annotation)),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| Self::annotation_type_is_unknown(&param.type_annotation))
                    || Self::annotation_type_is_unknown(returns)
            }
            TypeAnnotation::Generic { name, args } => {
                name.as_str() == "unknown" || args.iter().any(Self::annotation_type_is_unknown)
            }
            TypeAnnotation::Dyn(paths) => paths.iter().any(|path| path.as_str() == "unknown"),
            _ => false,
        }
    }

    // ADR-009 C3 S1c: visibility widened from private `fn` so the
    // `template_specialization` target glue can bind Sig types from the
    // AST/inference side (slice-0 report §7.4). Shared helper — NOT part of
    // the C3-G7 deletion set; behavior byte-unchanged.
    pub(in crate::compiler) fn annotation_param_type_annotation(
        &self,
        func_def: &FunctionDef,
        param_idx: usize,
        param: &shape_ast::ast::FunctionParameter,
    ) -> Option<TypeAnnotation> {
        if let Some(annotation) = param.type_annotation.as_ref() {
            return (!Self::annotation_type_is_unknown(annotation)).then(|| annotation.clone());
        }

        let shape_runtime::type_system::Type::Function { params, .. } =
            self.inference_facts.function_signature(&func_def.name)?
        else {
            return None;
        };
        let annotation = params.get(param_idx)?.to_annotation()?;
        (!Self::annotation_type_is_unknown(&annotation)).then_some(annotation)
    }

    pub(super) fn emit_annotation_lifecycle_calls(&mut self, func_def: &FunctionDef) -> Result<()> {
        if self.current_function.is_some() {
            return Ok(());
        }
        if func_def.annotations.is_empty() {
            return Ok(());
        }

        let self_fn_idx =
            self.find_function(&func_def.name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!(
                        "Internal error: function '{}' not found for annotation lifecycle dispatch",
                        func_def.name
                    ),
                    location: None,
                })? as u16;

        self.emit_annotation_lifecycle_calls_for_target(
            &func_def.annotations,
            &func_def.name,
            shape_ast::ast::functions::AnnotationTargetKind::Function,
            Some(self_fn_idx),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_type(
        &mut self,
        type_name: &str,
        annotations: &[shape_ast::ast::Annotation],
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            type_name,
            shape_ast::ast::functions::AnnotationTargetKind::Type,
            Some(0),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_module(
        &mut self,
        module_name: &str,
        annotations: &[shape_ast::ast::Annotation],
        target_id: Option<u16>,
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            module_name,
            shape_ast::ast::functions::AnnotationTargetKind::Module,
            target_id,
        )
    }

    fn emit_annotation_lifecycle_calls_for_target(
        &mut self,
        annotations: &[shape_ast::ast::Annotation],
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        for ann in annotations {
            let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else {
                continue;
            };

            if let Some(on_define_id) = compiled.on_define_handler {
                self.emit_annotation_handler_call(
                    on_define_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
            if let Some(metadata_id) = compiled.metadata_handler {
                self.emit_annotation_handler_call(
                    metadata_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
        }

        Ok(())
    }

    fn emit_annotation_handler_call(
        &mut self,
        handler_id: u16,
        annotation: &shape_ast::ast::Annotation,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let handler = self
            .program
            .functions
            .get(handler_id as usize)
            .cloned()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler function {} not found",
                    handler_id
                ),
                location: None,
            })?;
        let expected_base = 1 + annotation.args.len();
        let arity = handler.arity as usize;
        if arity < expected_base {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler '{}' arity {} is smaller than required base args {}",
                    handler.name, arity, expected_base
                ),
                location: None,
            });
        }

        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => {
                let id = target_id.ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: missing function id for annotation handler call"
                        .to_string(),
                    location: None,
                })?;
                let self_ref = self.program.add_constant(Constant::Number(id as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(self_ref)),
                ));
            }
            _ => {
                self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?;
            }
        }

        for ann_arg in &annotation.args {
            self.compile_expr(ann_arg)?;
        }

        for param_idx in expected_base..arity {
            let param_name = handler
                .param_names
                .get(param_idx)
                .map(|s| s.as_str())
                .unwrap_or_default();
            match param_name {
                "fn" | "target" => {
                    self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?
                }
                _ => {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
            }
        }

        let ac = self.program.add_constant(Constant::Int(arity as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(ac)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(handler_id))),
        ));
        self.record_blob_call(handler_id);
        self.emit(Instruction::simple(OpCode::Pop));
        Ok(())
    }

    fn annotation_target_kind_label(
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> &'static str {
        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => "function",
            shape_ast::ast::functions::AnnotationTargetKind::Type => "type",
            shape_ast::ast::functions::AnnotationTargetKind::Module => "module",
            shape_ast::ast::functions::AnnotationTargetKind::Expression => "expression",
            shape_ast::ast::functions::AnnotationTargetKind::Block => "block",
            shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr => "await_expr",
            shape_ast::ast::functions::AnnotationTargetKind::Binding => "binding",
        }
    }

    fn emit_annotation_target_descriptor(
        &mut self,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let name_const = self
            .program
            .add_constant(Constant::String(target_name.to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(name_const)),
        ));
        let kind_const = self.program.add_constant(Constant::String(
            Self::annotation_target_kind_label(target_kind).to_string(),
        ));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(kind_const)),
        ));
        if let Some(id) = target_id {
            let id_const = self.program.add_constant(Constant::Number(id as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(id_const)),
            ));
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        let fn_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("name", FieldType::String),
            ("kind", FieldType::String),
            ("id", FieldType::I64),
        ]);
        if fn_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation fn schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: fn_schema_id as u16,
                field_count: 3,
            }),
        ));
        Ok(())
    }

    /// Execute comptime annotation handlers for a function definition.
    ///
    /// When an annotation has a `comptime pre/post(...) { ... }` handler, self builds
    /// a ComptimeTarget from the function definition and executes the handler body
    /// at compile time with the target object bound to the handler parameter.
    pub(super) fn execute_comptime_handlers(
        &mut self,
        func_def: &mut FunctionDef,
        inferred_reference_optimizations: &[Option<ParamPassMode>],
        // ADR-009 C2 #13 (slice 4): out-parameter set to the node-borne
        // provenance of a `replace body` REPLACEMENT (if any handler emitted
        // one), so the D6 async-drop-context gate can authenticate the swapped
        // body it compiles under the user function name. Left `None` when no
        // `replace body` directive fired.
        replacement_body_origin: &mut Option<GeneratedNodeOrigin>,
    ) -> Result<bool> {
        assert_eq!(
            func_def.params.len(),
            inferred_reference_optimizations.len(),
            "comptime annotation provenance must stay slot-aligned"
        );
        let mut removed = false;
        let mut pending_original_body_shadow = None;
        // ADR-009 C3 #14 (slice 2, S2b): the per-target hook-install
        // accumulator — installs from EVERY handler run on this target
        // accumulate here in application order (the
        // `pending_original_body_shadow` threading pattern: a local threaded
        // as a parameter, never ambient state). S2c materializes the weave
        // ONCE from this accumulator after the last handler, wrapping the
        // final (possibly replace-body-edited) def; S2b stops at staged
        // installs + the journaled registry rows the apply seam wrote.
        let mut staged_hook_installs: Vec<StagedHookInstall> = Vec::new();
        let annotations = func_def.annotations.clone();

        // Phase 1: comptime pre
        for ann in &annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                if let Some(handler) = compiled.comptime_pre_handler {
                    // ADR-009 C3 #14 (slice 4): the def-param carrier reads
                    // the FULL param definitions (declared types ride along).
                    let def_params =
                        handler_resolution::annotation_def_params(&compiled.param_defs);
                    if self.execute_function_comptime_handler(
                        ann,
                        &handler,
                        &def_params,
                        func_def,
                        inferred_reference_optimizations,
                        &mut pending_original_body_shadow,
                        replacement_body_origin,
                        &mut staged_hook_installs,
                    )? {
                        removed = true;
                        break;
                    }
                }
            }
        }

        // Phase 2: comptime post
        if !removed {
            'post: for ann in &annotations {
                if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                    // Propagate the SAME `replace body` provenance out-param as
                    // phase 1: both handler phases route through the one shared
                    // `process_comptime_directives_for_function`, which is where
                    // a `replace body` directive is processed and its
                    // replacement origin recorded. A `replace body` is a
                    // post-handler directive today, but the directive processor
                    // is shared, so both phases must surface it uniformly — a
                    // discard here would silently drop the D6 gate's provenance
                    // for the one phase that actually emits it.
                    let def_params =
                        handler_resolution::annotation_def_params(&compiled.param_defs);
                    // ADR-009 C3 #14 (slice 4, S4c): the user comptime post
                    // handler runs FIRST, then the sugar lowering's
                    // SYNTHESIZED public-API handler (a TypedConfig
                    // definition's declarative hooks) — coexistence is
                    // allowed and ordered, matching the handler-map append
                    // order both pre-pass provenances use.
                    // S4c: the MINTED sugar body fns join the AST fn table
                    // (module-scope-shaped, C3-G3) before the handlers run,
                    // so the mono cache (`ensure_monomorphic_function`) can
                    // record + specialize them — the SAME `function_defs`
                    // contract hand-written and imported body fns already
                    // ride. Names are hygienic/unspellable (no user
                    // collision, unreachable from user code); `or_insert`
                    // keeps nested/re-entrant handler runs idempotent.
                    for def in &compiled.sugar_body_fns {
                        self.function_defs
                            .entry(def.name.clone())
                            .or_insert_with(|| def.clone());
                    }
                    let post_handlers = [
                        compiled.comptime_post_handler.clone(),
                        compiled.sugar_post_handler.clone(),
                    ];
                    for handler in post_handlers.into_iter().flatten() {
                        if self.execute_function_comptime_handler(
                            ann,
                            &handler,
                            &def_params,
                            func_def,
                            inferred_reference_optimizations,
                            &mut pending_original_body_shadow,
                            replacement_body_origin,
                            &mut staged_hook_installs,
                        )? {
                            removed = true;
                            break 'post;
                        }
                    }
                }
            }
        }

        if !removed && let Some(pending) = pending_original_body_shadow.take() {
            self.finalize_pending_original_body_shadow(pending)?;
        }

        // ADR-009 C3 #14 (slice 2, S2c): materialize the accumulated hook
        // installs ONCE, after the target's LAST handler + body directives
        // (and after a `replace body`'s original-body shadow finalized above,
        // so the weave wraps the FINAL — possibly replace-body-edited — def):
        // move the final body under the journaled hygienic weave shadow,
        // compile the shadow through the ordinary pipeline, and swap the
        // generated typed-AST wrapper into `func_def`, which then continues
        // through `compile_function_inner`'s ordinary tail (bytecode AND MIR
        // from the same wrapped definition — the C3-G6 SMALL shape). See
        // `template_specialization::weave` for the full contract.
        if !removed && !staged_hook_installs.is_empty() {
            self.materialize_hook_template_weave(func_def, &staged_hook_installs)?;
        }
        Ok(removed)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_function_comptime_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        annotation_def_params: &[(String, Option<TypeAnnotation>)],
        func_def: &mut FunctionDef,
        inferred_reference_optimizations: &[Option<ParamPassMode>],
        pending_original_body_shadow: &mut Option<PendingOriginalBodyShadow>,
        // ADR-009 C2 #13 (slice 4): threaded to the `replace body` directive
        // arm, which records the REPLACEMENT's node-borne provenance for the D6
        // gate (see `execute_comptime_handlers`).
        replacement_body_origin: &mut Option<GeneratedNodeOrigin>,
        // ADR-009 C3 #14 (slice 2, S2b): threaded to the `InstallHookTemplate`
        // apply seam (see `execute_comptime_handlers`).
        staged_hook_installs: &mut Vec<StagedHookInstall>,
    ) -> Result<bool> {
        // Build the target object from the function definition
        let target = super::comptime_target::ComptimeTarget::from_function(func_def);
        // ADR-009 D1 (S2): expansion site for this handler application.
        let expansion_site = self.annotation_expansion_site(annotation, handler, &target);
        // ADR-009 E1 #17 (slice 5): ONE overlay acquired before `to_nanboxed`
        // stamps the target's `type_ref` identities (producer stamp-gate) AND is
        // threaded (below) into `execute_comptime_annotation_handler` so the
        // consumer resolves composites off the same shared overlay memo.
        let freeze = self.comptime_freeze_overlay()?;
        // R8 W9 G.2 Step 2 Bucket 7: to_nanboxed now returns Result;
        // surface the V3-S5 ckpt-5 SURFACE through the caller's Result
        // chain instead of panicking.
        let target_value = target.to_nanboxed(Some(freeze.as_ref()))?;
        let target_name = func_def.name.clone();
        let handler_span = handler.span;
        let const_bindings = self
            .specialization_const_bindings
            .get(&target_name)
            .cloned()
            .unwrap_or_default();

        let mut execution = self.execute_comptime_annotation_handler(
            annotation,
            handler,
            target_value,
            annotation_def_params,
            &const_bindings,
            // Function target: no representation authority (Dec 56).
            None,
            freeze,
        )?;

        // ADR-009 E3 (S4, U11): resolve the `extend <target>` OWNER placeholder
        // against the handler's POSITION-0 target binding — replacing the deleted
        // magic `TypeName == "target"` literal substitution. A function target is
        // not a decomposable nominal type (`Opaque`); an `extend Widget { … }`
        // that names an explicit type (the v1 function-target extend surface)
        // does not match the position-0 binding and resolves nominally
        // untouched. The executed discovery pre-pass resolves identically.
        let owner = TargetOwner::new(target_name.clone(), NominalShape::Opaque);
        Self::resolve_extend_owner_placeholder(
            &mut execution.directives,
            &owner,
            handler.params.first().map(|p| p.name.as_str()),
        );

        self.process_comptime_directives_for_function(
            execution.directives,
            &target_name,
            func_def,
            &expansion_site,
            inferred_reference_optimizations,
            pending_original_body_shadow,
            replacement_body_origin,
            &annotation.name,
            staged_hook_installs,
        )
        .map_err(|e| {
            // ADR-009 D1 (S4): provenance-carrying generated-decl failures
            // pass through with their location notes intact.
            self.preserve_or_wrap_directive_failure(
                e,
                &format!("Comptime handler '{}'", annotation.name),
                handler_span,
            )
        })
    }

    // ABI flipped to `KindedSlot` per ADR-006 §2.7.10 / Q11 to align
    // with `super::comptime::execute_comptime_with_annotation_handler`
    // (compiler/comptime.rs:486) and the kinded replacement noted in
    // the prior SURFACE comment. The `comptime_builtins::ComptimeDirective::
    // SetParamValue { value: KindedSlot }` migration in
    // `compiler/comptime_builtins.rs:33` is the precedent.
    pub(super) fn execute_comptime_annotation_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        target_value: KindedSlot,
        // ADR-009 C3 #14 (slice 4): `(name, declared type annotation)` pairs
        // (`handler_resolution::annotation_def_params`); legacy defs carry
        // `None` throughout.
        annotation_def_params: &[(String, Option<TypeAnnotation>)],
        const_bindings: &[(String, KindedSlot)],
        // ADR-009 B5 (Dec 56): the annotated type's frozen identity halves for
        // declaration-attached TYPE-target handlers; `None` for function /
        // module / expression targets (which receive no representation
        // authority). The mint call is injected into the handler mini-VM.
        access_identity: Option<(i64, i64)>,
        // ADR-009 E1 #17 (slice 5): the SAME `Arc<FreezeOverlay>` the CALLER used
        // to stamp this target's `type_ref` identities (via `to_nanboxed`). It is
        // threaded straight to `execute_comptime_with_annotation_handler` so a
        // composite identity interned at stamp time is visible to the consumer's
        // `payload_of` off the shared memo — no second overlay is minted here.
        overlay: std::sync::Arc<super::comptime_builtins::FreezeOverlay>,
    ) -> Result<super::comptime::ComptimeExecutionResult> {
        let handler_span = handler.span;
        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: std::collections::HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let resolved_annotation_name = self.resolve_compiled_annotation_name(annotation);
        let defining_module_path = resolved_annotation_name
            .as_deref()
            .and_then(|name| name.rsplit_once("::").map(|(module_path, _)| module_path));
        let helper_authority =
            ComptimeHandlerHelperAuthority::for_compiled_name(resolved_annotation_name.as_deref());
        let comptime_helpers =
            self.collect_authorized_comptime_helpers(&handler.body, helper_authority);

        // §4.4: the comptime `ctx` compile-context (module_path + source file).
        let ctx_module_path = defining_module_path
            .map(str::to_string)
            .or_else(|| self.module_scope_stack.last().cloned())
            .unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        let context = format!("the @{} annotation handler", annotation.name);
        // ADR-009 §4.1 (S2): authoritative handler execution consumes the
        // per-compilation-unit freeze handle — the empty-snapshot defect
        // (`TypeReflectionSnapshot::default()`) is deleted. This runs in
        // pass 2, after the freeze barrier; a handler reached without an
        // installed freeze is a compile error (row 3).
        //
        // ADR-009 E1 #17 (slice 5): the handle is now the caller-supplied
        // `overlay` — the SAME `Arc` used to stamp this target's `type_ref`
        // identities — not a freshly minted one, so the stamp and resolve share
        // one composite memo.
        //
        // ADR-009 C3 #14 (slice 2): the hook-template body-fn lookup — the
        // SAME AST fn table `collect_authorized_comptime_helpers` reads
        // (`self.function_defs`; bare name first, then qualified under the
        // handler's defining module), threaded as a PARAMETER into the
        // executor's emit-side rewrite. Never ambient state.
        //
        // S4c: the annotation's STORED sugar body fns (installer-attached to
        // the `CompiledAnnotation` carrier) resolve FIRST — this is how the
        // synthesized sugar post handler reaches its minted hook bodies at
        // pass-2 (hygienic names — no user fn can collide).
        let sugar_body_fns: Vec<FunctionDef> = self
            .lookup_compiled_annotation(annotation)
            .map(|(_, compiled)| compiled.sugar_body_fns)
            .unwrap_or_default();
        let function_defs = &self.function_defs;
        let template_body_fn_lookup = move |name: &str| -> Option<FunctionDef> {
            sugar_body_fns
                .iter()
                .find(|def| def.name == name)
                .cloned()
                .or_else(|| function_defs.get(name).cloned())
                .or_else(|| {
                    defining_module_path.and_then(|module| {
                        function_defs
                            .get(&Self::qualify_module_symbol(module, name))
                            .cloned()
                    })
                })
        };
        // ADR-009 C3 #14 (slice 5, S5a) — the [C0931] Dec-65 config-arg
        // pre-check at the authoritative pass-2 seam (the pre-pass seams
        // check too; module bindings registered between phases make this
        // one the totality anchor). Injected specialization
        // `const_bindings` are exempt by name.
        self.reject_runtime_module_binding_config_args(annotation, const_bindings)?;
        let execution = super::comptime::execute_comptime_with_annotation_handler(
            &handler.body,
            &handler.params,
            target_value,
            &annotation.args,
            annotation_def_params,
            const_bindings,
            &comptime_helpers,
            &extensions,
            known_type_symbols,
            &ctx_module_path,
            &ctx_file,
            trait_impls,
            overlay,
            // ADR-009 B5 (Dec 56): forward the caller-supplied type identity
            // (Some for a declaration-attached type-target hook; None otherwise).
            access_identity,
            &template_body_fn_lookup,
        )
        .map_err(|e| self.build_comptime_failure(&e, handler_span, &context))?;
        // §4.4: re-emit any `warning()` output anchored at this handler site.
        self.surface_comptime_warnings(&execution.warnings, handler_span);
        Ok(execution)
    }

    fn collect_scoped_names_in_statement(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::Return(Some(expr), _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Statement::Assignment(assign, _) => {
                Self::collect_scoped_names_in_expr(&assign.value, names)
            }
            Statement::Expression(expr, _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::For(loop_expr, _) => {
                match &loop_expr.init {
                    shape_ast::ast::ForInit::ForIn { iter, .. } => {
                        Self::collect_scoped_names_in_expr(iter, names);
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::collect_scoped_names_in_statement(init, names);
                        Self::collect_scoped_names_in_expr(condition, names);
                        Self::collect_scoped_names_in_expr(update, names);
                    }
                }
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::While(loop_expr, _) => {
                Self::collect_scoped_names_in_expr(&loop_expr.condition, names);
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::If(if_stmt, _) => {
                Self::collect_scoped_names_in_expr(&if_stmt.condition, names);
                for body_stmt in &if_stmt.then_body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for body_stmt in else_body {
                        Self::collect_scoped_names_in_statement(body_stmt, names);
                    }
                }
            }
            Statement::SetReturnExpr { expression, .. }
            | Statement::SetParamTypeExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                Self::collect_scoped_names_in_expr(expression, names);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            _ => {}
        }
    }

    fn collect_scoped_names_in_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::MethodCall {
                receiver,
                method,
                args,
                named_args,
                ..
            } => {
                if let Expr::Identifier(namespace, _) = receiver.as_ref() {
                    names.insert(format!("{}::{}", namespace, method));
                }
                Self::collect_scoped_names_in_expr(receiver, names);
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::FunctionCall {
                const_args,
                args,
                named_args,
                ..
            } => {
                handler_resolution::seed_function_call(expr, names);
                for arg in const_args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::QualifiedFunctionCall {
                const_args,
                args,
                named_args,
                ..
            } => {
                handler_resolution::seed_function_call(expr, names);
                for arg in const_args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::collect_scoped_names_in_expr(left, names);
                Self::collect_scoped_names_in_expr(right, names);
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::Reference { expr: operand, .. }
            | Expr::AsyncScope(operand, _)
            | Expr::DataRelativeAccess {
                reference: operand, ..
            } => {
                Self::collect_scoped_names_in_expr(operand, names);
            }
            Expr::PropertyAccess { object, .. } => {
                Self::collect_scoped_names_in_expr(object, names)
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::collect_scoped_names_in_expr(object, names);
                Self::collect_scoped_names_in_expr(index, names);
                if let Some(end) = end_index {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_scoped_names_in_expr(condition, names);
                Self::collect_scoped_names_in_expr(then_expr, names);
                if let Some(else_expr) = else_expr {
                    Self::collect_scoped_names_in_expr(else_expr, names);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } | ObjectEntry::Spread(value) => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
            }
            Expr::Array(values, _) => {
                for value in values {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::ListComprehension(comp, _) => {
                Self::collect_scoped_names_in_expr(&comp.element, names);
                for clause in &comp.clauses {
                    Self::collect_scoped_names_in_expr(&clause.iterable, names);
                    if let Some(filter) = &clause.filter {
                        Self::collect_scoped_names_in_expr(filter, names);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                Self::collect_scoped_names_in_expr(value, names);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            Self::collect_scoped_names_in_expr(&assign.value, names);
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            Self::collect_scoped_names_in_statement(stmt, names);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                    }
                }
            }
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                Self::collect_scoped_names_in_expr(expr, names);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => Self::collect_scoped_names_in_expr(expr, names),
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_scoped_names_in_expr(&if_expr.condition, names);
                Self::collect_scoped_names_in_expr(&if_expr.then_branch, names);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_scoped_names_in_expr(else_branch, names);
                }
            }
            Expr::While(while_expr, _) => {
                Self::collect_scoped_names_in_expr(&while_expr.condition, names);
                Self::collect_scoped_names_in_expr(&while_expr.body, names);
            }
            Expr::For(for_expr, _) => {
                Self::collect_scoped_names_in_expr(&for_expr.iterable, names);
                Self::collect_scoped_names_in_expr(&for_expr.body, names);
            }
            Expr::Loop(loop_expr, _) => Self::collect_scoped_names_in_expr(&loop_expr.body, names),
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
                Self::collect_scoped_names_in_expr(&let_expr.body, names);
            }
            Expr::Assign(assign_expr, _) => {
                Self::collect_scoped_names_in_expr(&assign_expr.target, names);
                Self::collect_scoped_names_in_expr(&assign_expr.value, names);
            }
            Expr::Break(Some(value), _) | Expr::Return(Some(value), _) => {
                Self::collect_scoped_names_in_expr(value, names);
            }
            Expr::Match(match_expr, _) => {
                Self::collect_scoped_names_in_expr(&match_expr.scrutinee, names);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        Self::collect_scoped_names_in_expr(guard, names);
                    }
                    Self::collect_scoped_names_in_expr(&arm.body, names);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    Self::collect_scoped_names_in_expr(start, names);
                }
                if let Some(end) = end {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::TimeframeContext { expr, .. } | Expr::UsingImpl { expr, .. } => {
                Self::collect_scoped_names_in_expr(expr, names);
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::WindowExpr(window_expr, _) => {
                use shape_ast::ast::WindowFunction;

                match &window_expr.function {
                    WindowFunction::Lag { expr, default, .. }
                    | WindowFunction::Lead { expr, default, .. } => {
                        Self::collect_scoped_names_in_expr(expr, names);
                        if let Some(default) = default {
                            Self::collect_scoped_names_in_expr(default, names);
                        }
                    }
                    WindowFunction::FirstValue(expr)
                    | WindowFunction::LastValue(expr)
                    | WindowFunction::Sum(expr)
                    | WindowFunction::Avg(expr)
                    | WindowFunction::Min(expr)
                    | WindowFunction::Max(expr) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::NthValue(expr, _) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(Some(expr)) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(None)
                    | WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Ntile(_) => {}
                }

                for expr in &window_expr.over.partition_by {
                    Self::collect_scoped_names_in_expr(expr, names);
                }
                if let Some(order_by) = &window_expr.over.order_by {
                    for (expr, _) in &order_by.columns {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                }
            }
            Expr::FromQuery(from_query, _) => {
                Self::collect_scoped_names_in_expr(&from_query.source, names);
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                        shape_ast::ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                Self::collect_scoped_names_in_expr(&spec.key, names);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            Self::collect_scoped_names_in_expr(element, names);
                            Self::collect_scoped_names_in_expr(key, names);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            Self::collect_scoped_names_in_expr(source, names);
                            Self::collect_scoped_names_in_expr(left_key, names);
                            Self::collect_scoped_names_in_expr(right_key, names);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
                Self::collect_scoped_names_in_expr(&from_query.select, names);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    Self::collect_scoped_names_in_expr(&branch.expr, names);
                    for ann in &branch.annotations {
                        for arg in &ann.args {
                            Self::collect_scoped_names_in_expr(arg, names);
                        }
                    }
                }
            }
            Expr::Annotated {
                annotation, target, ..
            } => {
                for arg in &annotation.args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                Self::collect_scoped_names_in_expr(target, names);
            }
            Expr::AsyncLet(async_let, _) => {
                Self::collect_scoped_names_in_expr(&async_let.expr, names)
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::ComptimeFor(comptime_for, _) => {
                Self::collect_scoped_names_in_expr(&comptime_for.iterable, names);
                for stmt in &comptime_for.body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            },
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for elem in row {
                        Self::collect_scoped_names_in_expr(elem, names);
                    }
                }
            }
            // ADR-009 C3 #14 (slice 5, S5b): a VALUE-position install-family
            // reference (`let f = before_hook`) is recorded under the
            // unspellable [`INSTALL_FAMILY_VALUE_MARK`] so the static C3-G8
            // scan sees it (P-G8d measured that shape SILENT on a generic
            // target). The mark can never resolve in any fn table, so
            // helper collection is byte-equivalent to before; every OTHER
            // identifier stays uncollected (the pre-S5b leaf behavior).
            Expr::Identifier(name, _) if Self::is_install_family_name(name) => {
                names.insert(format!("{INSTALL_FAMILY_VALUE_MARK}{name}"));
            }
            Expr::Literal(..)
            | Expr::Identifier(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::DateTime(..)
            | Expr::PatternRef(..)
            | Expr::Duration(..)
            | Expr::Break(None, _)
            | Expr::Return(None, _)
            | Expr::Continue(..)
            // ADR-009 A2: type syntax is a leaf — no scoped names inside.
            | Expr::TypeSyntax(..)
            | Expr::Unit(..) => {}
        }
    }

    /// ADR-009 D1 (S4): the compiler-owned generated-symbol query surface —
    /// the ONE query API of spec §4.1 for generated declarations. Tooling
    /// (the LSP in slice S5, diagnostics here in S4) resolves generated
    /// symbols to `{SymbolId, checked-decl location, application location,
    /// generator-definition location}` and lists them for workspace-symbol
    /// consumption THROUGH this handle, answered from the S2 identity table
    /// only — never by text scan, never by a second expansion run
    /// (Decision 66 closing rule).
    ///
    /// Query the compiler AFTER compilation (`compile_in_place`) so the
    /// table holds every reserved expansion of the unit.
    pub fn generated_symbol_query(&self) -> &GeneratedSymbolTable {
        &self.generated_symbols
    }

    /// ADR-009 E3 (slice S1): the generated analysis items (`Item::Extend` /
    /// `Item::Function`) materialized by the executed declaration-discovery
    /// pre-pass for this compilation unit. Empty until `compile_in_place`
    /// runs. This is the executed authority that replaced the deleted
    /// non-evaluating static AST scan; static consumers augment their program
    /// view from this slice.
    pub fn generated_analysis_items(&self) -> &[shape_ast::ast::Item] {
        &self.generated_analysis_items
    }

    /// ADR-009 D1 (S2): build the [`ExpansionSite`] for one comptime
    /// annotation-handler application. Called by BOTH phases of the existing
    /// extend/materialization path — the speculative pre-pass
    /// (`materialize_computed_comptime_extends`) and the authoritative
    /// pass-2 handler execution sites — from the SAME AST inputs, so the two
    /// runs of one application agree on one `ExpansionIdentity` (risk 7:
    /// provenance must not double).
    pub(super) fn annotation_expansion_site(
        &self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        target: &super::comptime_target::ComptimeTarget,
    ) -> ExpansionSite {
        let file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("");
        let generator = GeneratorRef::from_canonical_descriptor(format!(
            "annotation:{}:{}",
            annotation.name,
            annotation_handler_kind_descriptor(&handler.handler_type)
        ));
        let application = ApplicationId::from_canonical_descriptor(format!(
            "application:{}:{}:{}",
            file, annotation.span.start, annotation.span.end
        ));
        let target_identity = TargetIdentity::from_canonical_descriptor(format!(
            "{}:{}",
            annotation_target_kind_descriptor(target.kind),
            target.name
        ));
        let argument_descriptors: Vec<(String, String)> = annotation
            .args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                (
                    index.to_string(),
                    super::comptime_target::expr_to_string_lossy(arg),
                )
            })
            .collect();
        let argument_refs: Vec<(&str, &str)> = argument_descriptors
            .iter()
            .map(|(name, descriptor)| (name.as_str(), descriptor.as_str()))
            .collect();
        let dependency_descriptors = comptime_target_dependency_descriptors(target);
        let dependency_refs: Vec<&str> =
            dependency_descriptors.iter().map(String::as_str).collect();
        ExpansionSite::new(
            ExpansionIdentity::new(
                generator,
                application,
                target_identity,
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&argument_refs),
                CanonicalHash::from_canonical_dependency_descriptors(&dependency_refs),
            ),
            self.current_file_id,
            annotation.span,
            // Generator-definition anchor: the annotation's comptime
            // handler span (S4 query surface + row-7 diagnostics answer
            // "generator defined here" from this anchor).
            handler.span,
        )
    }

    /// ADR-009 D1 (S2): build the [`ExpansionSite`] for a `comptime { }`
    /// block emitting directives. The block is its own generator AND its own
    /// application site (there is no separate annotation application).
    pub(super) fn comptime_block_expansion_site(
        &self,
        span: Span,
        module_path: &str,
    ) -> ExpansionSite {
        let file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("");
        let block_descriptor = format!("comptime-block:{}:{}:{}", file, span.start, span.end);
        let no_arguments: [(&str, &str); 0] = [];
        ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor(block_descriptor.clone()),
                ApplicationId::from_canonical_descriptor(format!(
                    "application:{}:{}:{}",
                    file, span.start, span.end
                )),
                TargetIdentity::from_canonical_descriptor(format!("module:{module_path}")),
                ComptimeStage::ModuleComptimeBlock,
                CanonicalHash::from_canonical_argument_descriptors(&no_arguments),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            self.current_file_id,
            span,
            // The comptime block is its own generator AND its own
            // application site: one span fills both anchor roles.
            span,
        )
    }

    /// Map a reservation-layer rejection (rows 1/2/3, a `String` carrying
    /// the named diagnostic + provenance rendering) into a spanned compile
    /// error anchored at the application site.
    fn expansion_rejection(&self, message: String, site: &ExpansionSite) -> ShapeError {
        ShapeError::SemanticError {
            message,
            location: Some(self.span_to_source_location(site.application_span())),
        }
    }

    /// ADR-009 E3 (S4, U11): resolve the `extend <target>` OWNER placeholder in
    /// a batch of handler-emitted comptime directives, in place.
    ///
    /// This replaces the deleted magic `TypeName == "target"` literal
    /// substitution. An `extend` directive whose head names the handler's
    /// POSITION-0 target binding (`target_binding` — the handler's first
    /// parameter, bound by POSITION, never by a fixed `"target"` spelling) is
    /// resolved to `owner` (the annotated nominal type, [`TargetOwner::name`]).
    /// Any OTHER head — including a user type literally named `target` when the
    /// handler's first parameter is spelled differently — is left untouched and
    /// resolves NOMINALLY through the ordinary type-name table. No `"target"`
    /// string enters a symbol table by magic; only free `extend`
    /// (`ComptimeDirective::Extend`) directives are placeholder-bearing (the
    /// computed-snippet `ExtendItems` form already carries the real interpolated
    /// name, e.g. `extend {target.name} { … }`).
    ///
    /// Both the executed declaration-discovery pre-pass and the authoritative
    /// pass-2 compile call this with the SAME handler (hence the same
    /// position-0 binding) and the same owner, so both phases produce the
    /// identical resolved `extend` and reserve ONE expansion identity per
    /// generated method.
    pub(super) fn resolve_extend_owner_placeholder(
        directives: &mut [super::comptime_builtins::ComptimeDirective],
        owner: &TargetOwner,
        target_binding: Option<&str>,
    ) {
        let Some(binding) = target_binding else {
            return;
        };
        for directive in directives.iter_mut() {
            let super::comptime_builtins::ComptimeDirective::Extend(extend) = directive else {
                continue;
            };
            let head = match &extend.type_name {
                shape_ast::ast::TypeName::Simple(name) => name,
                shape_ast::ast::TypeName::Generic { name, .. } => name,
            };
            if head == binding {
                match &mut extend.type_name {
                    shape_ast::ast::TypeName::Simple(name) => *name = owner.name().into(),
                    shape_ast::ast::TypeName::Generic { name, .. } => *name = owner.name().into(),
                }
            }
        }
    }

    pub(super) fn apply_comptime_extend(
        &mut self,
        mut extend: shape_ast::ast::ExtendStatement,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<()> {
        // ADR-009 E3 (S4, U11): the `extend <target>` OWNER placeholder is
        // resolved upstream, at the handler-execution site, by
        // `resolve_extend_owner_placeholder` (position-0 binding against the
        // typed `TargetOwner`). The deleted magic `TypeName == "target"` literal
        // substitution formerly lived HERE; `extend.type_name` now already
        // carries the resolved nominal owner name (or a real user type name that
        // resolves nominally).

        // ADR-009 D1 (S2), rejection row 1: a generated declaration must
        // anchor at a real application span — refused HERE, before any
        // registration or compilation (Dec 68 required rejection). The
        // generator-definition anchor is held to the same rule (S4).
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;

        // Row-3 content fingerprints are taken over the handler-emitted
        // method AST (post target-substitution, PRE parameter-annotation
        // enrichment) so the pre-pass and pass-2 encodings agree.
        let method_contents: Vec<CanonicalHash> = extend
            .methods
            .iter()
            .map(|method| generated_extend_method_content(&extend.type_name, method))
            .collect();

        self.annotate_comptime_extend_method_params(&mut extend.methods, target_name);

        let extend_type_str = match &extend.type_name {
            shape_ast::ast::TypeName::Simple(name) => name.clone(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
        };

        for (method, content) in extend.methods.iter().zip(method_contents) {
            let mut func_def = self.desugar_extend_method(method, &extend.type_name)?;
            // ADR-009 D1 (S3): the registered decl anchors at the real
            // application span (content fingerprint above is over the raw
            // handler-emitted AST — see `anchor_generated_function_decl`).
            anchor_generated_function_decl(&mut func_def, site.application_span());
            let node_path = GeneratedNodePath::decl_root(format!("extend:{extend_type_str}"))
                .child(format!("method:{}", method.name));
            let origin = GeneratedOrigin {
                expansion: site.identity().clone(),
                node_path: node_path.clone(),
                source_anchor,
            };
            self.stamp_generated_closure_provenance(&mut func_def.body, &origin, &func_def.name);
            // §4.9.1 + D1 identity-keyed dedup: if the whole-program
            // pre-pass already reserved this identity and registered the
            // method's SIGNATURE (so it is visible to the analyzer, method
            // dispatch, and every user body), the reservation is re-issued —
            // skip re-registering (a second `register_function` would create
            // a duplicate slot). The body is still compiled below, filling
            // the pre-registered slot, so the method is compiled exactly
            // once through the identical path.
            match self.reserve_generated_decl_journaled(
                &func_def.name,
                origin.clone(),
                content,
                generator_anchor,
            ) {
                Ok(SymbolReservation::Fresh(_)) => {
                    // ADR-009 D1 (S4), rejection row 7: a diagnostic raised
                    // on a generated declaration carries generated-node +
                    // application + generator locations.
                    self.register_function(&func_def).map_err(|e| {
                        self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                    })?;
                }
                Ok(SymbolReservation::Reissued(_)) => {}
                Err(message) => return Err(self.expansion_rejection(message, site)),
            }
            // ADR-009 C2 #13 (slice 2, battery row 10b; slice 4 threading): arm
            // the D6 async-drop-context gate for this generated body by handing
            // its node-borne provenance to the compile as a PARAMETER (no shared
            // compiler field — a nested monomorphization compile can never steal
            // it). The gate is evaluated at the end of the inner compile on the
            // authenticated body, using the RAII drop-plan's emission-authority
            // drop signal, so a rejection surfaces through the same
            // `build_generated_decl_failure` wrap as any body error and rolls
            // back atomically in the driver-level install transaction.
            let node_origin = origin.to_node_origin(&self.generated_node_issuer, &func_def.name);
            // Wave-38F generated-method JIT parity: hand-written `extend`
            // methods compile through the full driver (`compile_function`),
            // which lowers MIR and back-patches `Function.mir_data` for the
            // JIT. Generated methods need the same path; the signature was
            // already registered above/pre-pass, so this only fills the body
            // and MIR for the existing function slot.
            //
            // ADR-009 D1 (S4), rejection row 7: a body error inside the
            // generated method surfaces with full expansion provenance —
            // generated-node + application-site + generator-definition
            // locations — never as a bare error pointing at handler-emitted
            // offsets.
            self.compile_function_with_generated_origin(&func_def, Some(node_origin))
                .map_err(|e| {
                    self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                })?;
        }
        Ok(())
    }

    /// §4.5.7: apply a computed `extend (expr)` directive — register + compile
    /// the generated items additively at the annotated item's module scope.
    /// Free functions and `extend Type { ... }` blocks are the v1 surface (the
    /// two shapes the derive/LLM showcases emit); other top-level item kinds
    /// surface a clean compile error rather than a partial implementation.
    ///
    /// Function signatures are registered in a first pass so generated items may
    /// reference one another, then bodies are compiled — the same two-phase
    /// shape the top-level pipeline uses.
    /// Comptime-excellence §4.5.1 whole-program pre-pass.
    ///
    /// A computed `extend (item_fn(...))` directive inside a comptime annotation
    /// handler only materializes its generated free function during pass-2,
    /// when the annotated *type* is compiled — which is *after* the analyzer
    /// and after user function bodies resolve their call sites. So a program
    /// with `fn main() { print(User_json_schema()) }` failed with "Undefined
    /// function", even though the same call at top level worked, because the
    /// generated `User_json_schema` was invisible to every earlier phase.
    ///
    /// This pre-pass runs the type-targeting comptime handlers *before* the
    /// analyzer, materializes the generated declarations, and returns the
    /// generated free functions so the driver can insert them as ordinary program
    /// items. From
    /// there they flow through function registration, analysis, inference and
    /// pass-2 body compilation exactly like hand-written functions — visible to
    /// `fn main()` and to every user body.
    ///
    /// The pre-pass is speculative: any handler that fails here (missing
    /// helper, `error()` on a non-serializable field, etc.) is silently
    /// skipped — pass-2 re-runs the same handler authoritatively and surfaces
    /// the real diagnostic with its proper span. Every declaration it does
    /// materialize is reserved in the compiler's `GeneratedSymbolTable`
    /// under its `ExpansionIdentity` (ADR-009 D1) so pass-2's
    /// `apply_comptime_extend_items` re-issues the same reservation instead
    /// of registering it a second time.
    ///
    /// Both generated free functions and generated type-extension methods
    /// (`extend Type { method ... }`, §4.9.1) are hoisted: the extend's method
    /// signatures are registered here and the `extend` block is returned so the
    /// analyzer and method-dispatch resolution learn the method on the type
    /// before any user body compiles. Pass-2's `apply_comptime_extend` compiles
    /// each pre-registered method body.
    pub(super) fn materialize_computed_comptime_extends(
        &mut self,
        program: &shape_ast::ast::Program,
    ) -> Result<Vec<shape_ast::ast::Item>> {
        use shape_ast::ast::Item;

        // annotation bare-name -> (comptime handlers, annotation-def param names)
        let handler_map = self.collect_comptime_annotation_handlers(program)?;
        if handler_map.is_empty() {
            return Ok(Vec::new());
        }

        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();

        // Snapshot the complete source-executable v1 annotation frontier in
        // pass-2 lexical form. The collector recurses inline modules, unwraps
        // exported functions/structs, and lowers source Extend/Impl methods via
        // the same compiler desugarers pass 2 uses.
        let discovery_seed = self.collect_declaration_discovery_targets(program)?;

        // Known type symbols: every discovered source struct plus every type
        // symbol already registered (imported modules compiled in graph phase
        // 1). Nested/exported types therefore enter handler execution under
        // the same qualified identity pass 2 observes.
        let mut known_type_symbols: HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        for target in &discovery_seed {
            if matches!(target, DeclarationDiscoveryTarget::Struct { .. }) {
                known_type_symbols.insert(target.name().to_string());
            }
        }

        let ctx_module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        let mut generated: Vec<Item> = Vec::new();
        // ADR-009 E2 #18 (slice 3): fresh per-run set of const-free function-target
        // `replace body` edits materialized below (drained by the driver into the
        // analysis-program clone). Cleared here so a reused compiler never carries
        // a prior compile's edits.
        self.pending_replace_body_analysis.clear();

        // ADR-009 D2 (Decision 67): the monotonic declaration-discovery fixed
        // point. The formerly single, unbounded speculative pass is now a
        // bounded worklist that reaches a fixed point BEFORE the analyzer runs
        // (this method is still invoked exactly once — the fixed point is the
        // SINGLE discovery pass, no speculative second evaluation). Each round
        // drains the worklist of struct definitions, runs every not-yet-run
        // annotation application once (run-once memo keyed on the full
        // `ExpansionIdentity` = ApplicationId + dependencies hash), records the
        // generated headers immutably, and enqueues any newly generated
        // annotated type for the next round (additions-only). The v1 directive
        // surface emits only free functions and `extend` methods — never a new
        // annotated type — so real programs converge in one round; the worklist
        // machinery makes multi-level discovery total and its rejections named.
        let mut discovery = DeclarationDiscoveryFixedPoint::new();
        let mut worklist: Vec<DeclarationDiscoveryTarget> = discovery_seed;
        // Generated annotated type → the application whose expansion produced
        // it (the output-triggers edge source for cycle detection).
        let mut type_producer: HashMap<String, ExpansionIdentity> = HashMap::new();

        while !worklist.is_empty() {
            // Round bound (DISCOVERY_UNBOUNDED on overflow).
            discovery
                .begin_round()
                .map_err(|message| self.build_discovery_failure(message, None))?;
            let round_defs = std::mem::take(&mut worklist);
            // Frontier state for the monotone-convergence (oscillation) guard:
            // the sorted set of target names discovered/pending this round.
            let mut frontier: Vec<String> =
                round_defs.iter().map(|d| d.name().to_string()).collect();
            frontier.sort();
            discovery
                .observe_round_state(&frontier)
                .map_err(|message| self.build_discovery_failure(message, None))?;
            // Types generated this round, re-scanned next round (additions-only;
            // discovered headers stay immutable through discovery).
            let mut newly_generated_types: Vec<DeclarationDiscoveryTarget> = Vec::new();

            for disc_target in &round_defs {
                for ann in disc_target.annotations() {
                    let Some((_, entry)) = self.resolve_comptime_annotation_handlers(
                        &handler_map,
                        ann,
                        disc_target.lexical_module_path(),
                    ) else {
                        continue;
                    };
                    for handler in &entry.handlers {
                        // Per-kind target construction. A TYPE target builds the
                        // field-carrying `ComptimeTarget` and (Dec 56) names the
                        // annotated type for representation-authority minting; a
                        // FUNCTION target builds from the signature and receives no
                        // authority (`access_type_name = None`). Everything below
                        // is shared.
                        let (target, access_type_name) = disc_target.comptime_target();
                        // ADR-009 D1 (S2): the pre-pass builds the SAME expansion
                        // site pass-2 will build for this application (same ann
                        // node, same handler AST, same ComptimeTarget inputs), so
                        // both phases reserve one identity per generated decl.
                        let expansion_site = self.annotation_expansion_site(ann, handler, &target);
                        // ADR-009 D2 (Decision 67): output-triggers edge for cycle
                        // detection — if this target was itself generated by an
                        // earlier expansion, record the producing application →
                        // this application edge (DISCOVERY_CYCLE on a closing edge).
                        if let Some(producer) = type_producer.get(disc_target.name()) {
                            discovery
                                .record_trigger(producer, expansion_site.identity())
                                .map_err(|message| {
                                    self.build_discovery_failure(message, Some(&expansion_site))
                                })?;
                        }
                        // ADR-009 D2 run-once memo: run each application exactly
                        // once per (ApplicationId + dependencies hash). A re-claimed
                        // identity (the struct re-enqueued in a later round)
                        // short-circuits — this is memoization, NOT a silent
                        // failure skip (DISCOVERY_UNBOUNDED on the expansion bound).
                        match discovery.claim(expansion_site.identity()) {
                            Ok(ApplicationClaim::Fresh) => {}
                            Ok(ApplicationClaim::AlreadyApplied) => continue,
                            Err(message) => {
                                return Err(
                                    self.build_discovery_failure(message, Some(&expansion_site))
                                );
                            }
                        }
                        // S3 pre-pass freeze rule (see `s3_freeze_gate_tests`
                        // module doc): this speculative run fires AFTER the
                        // semantic-freeze barrier and consumes the real
                        // registration-complete freeze handle. A site that
                        // cannot obtain the handle is the row-3 named compile
                        // error; the handle is acquired before the
                        // output-suppression toggle so the error path cannot leak
                        // suppression state.
                        //
                        // ADR-009 E1 #17 (slice 5): acquired BEFORE `to_nanboxed`
                        // so the SAME `Arc<FreezeOverlay>` both stamps the
                        // target's `type_ref` identities (producer stamp-gate,
                        // through the `let Ok(..) else continue` swallow — a
                        // canonicalize gap stays INVALID, never propagates) AND is
                        // threaded to the handler executor below (shared
                        // composite-memo round-trip) and to `identity_of`.
                        let freeze = self.comptime_freeze_overlay()?;
                        let Ok(target_value) = target.to_nanboxed(Some(freeze.as_ref())) else {
                            continue;
                        };

                        // Reachable comptime helpers for this handler body. At
                        // pre-pass time `function_defs` already holds every
                        // dependency-module function (graph phase 1); root helpers
                        // that are not yet registered simply fall back to pass-2.
                        let handler_module_path = entry
                            .defining_module_path
                            .as_deref()
                            .or_else(|| disc_target.lexical_module_path())
                            .unwrap_or(&ctx_module_path);
                        let helpers = self.collect_authorized_comptime_helpers(
                            &handler.body,
                            entry.helper_authority(),
                        );

                        // §4.5.1: this pre-pass run is speculative (it only
                        // materializes generated function signatures); pass-2
                        // re-runs the same handler authoritatively. Suppress raw
                        // handler output during the speculative run so a handler
                        // that prints does not emit twice. Reflection-using
                        // handlers materialize their generated functions here
                        // (visible to every user body) instead of deferring to
                        // pass 2. The freeze handle was acquired above (before
                        // `to_nanboxed`); it is reused here — one acquisition.
                        // ADR-009 B5 (Dec 56): a declaration-attached TYPE-target
                        // hook mints a `RepresentationAccess<T>` authority bound to
                        // the annotated type's frozen identity and delivers it as
                        // the handler's third positional `access` parameter (author
                        // consent). A type whose identity the freeze never issued
                        // mints no authority (`None`). FUNCTION targets
                        // (`access_type_name = None`) receive no authority.
                        let access_identity = access_type_name
                            .as_deref()
                            .and_then(|name| freeze.identity_of(name))
                            .map(|identity| (identity.high, identity.low));
                        // ADR-009 C3 #14 (slice 2): the hook-template body-fn
                        // lookup (same table, threaded as a parameter; a
                        // pre-pass miss defers to pass-2 like every other
                        // pre-pass limitation below).
                        //
                        // S4c: entry-minted sugar body fns resolve FIRST.
                        let function_defs = &self.function_defs;
                        let sugar_body_fns = &entry.sugar_body_fns;
                        let template_body_fn_lookup =
                            move |name: &str| -> Option<FunctionDef> {
                                sugar_body_fns
                                    .iter()
                                    .find(|def| def.name == name)
                                    .cloned()
                                    .or_else(|| function_defs.get(name).cloned())
                                    .or_else(|| {
                                        function_defs
                                            .get(&Self::qualify_module_symbol(
                                                handler_module_path,
                                                name,
                                            ))
                                            .cloned()
                                    })
                            };
                        // ADR-009 C3 #14 (slice 5, S5a) — the [C0931]
                        // Dec-65 config-arg pre-check: Err BEFORE execution
                        // so the pre-pass-limitation swallow below cannot
                        // eat it.
                        self.reject_runtime_module_binding_config_args(ann, &[])?;
                        let prev_suppressed =
                            super::comptime_builtins::set_comptime_output_suppressed(true);
                        let execution_result =
                            super::comptime::execute_comptime_with_annotation_handler(
                                &handler.body,
                                &handler.params,
                                target_value,
                                &ann.args,
                                &entry.def_params,
                                &[],
                                &helpers,
                                &extensions,
                                known_type_symbols.clone(),
                                handler_module_path,
                                &ctx_file,
                                trait_impls.clone(),
                                freeze,
                                access_identity,
                                &template_body_fn_lookup,
                            );
                        super::comptime_builtins::set_comptime_output_suppressed(prev_suppressed);
                        let mut execution = match execution_result {
                            Ok(execution) => execution,
                            Err(e) => {
                                // A genuine user `error()` call in the handler is a
                                // deterministic compile error — surface it here with
                                // a clean, spanned, LSDS-routed diagnostic anchored
                                // at the annotation application site (§4.4). If we
                                // swallowed it, the analyzer would instead reject the
                                // never-generated function with a confusing
                                // "Undefined function" and mask the real cause.
                                //
                                // Any other failure is treated as a pre-pass
                                // limitation (e.g. a helper only registered later)
                                // and deferred to pass-2, which re-runs the handler
                                // authoritatively.
                                if e.to_string().contains("[comptime error]") {
                                    let context = format!(
                                        "the @{} annotation on {}",
                                        ann.name,
                                        disc_target.name()
                                    );
                                    return Err(self.build_comptime_failure(&e, ann.span, &context));
                                }
                                continue;
                            }
                        };

                        // ADR-009 E3 (S4, U11): resolve the `extend <target>`
                        // OWNER placeholder against the handler's POSITION-0
                        // target binding and the TYPED owner descriptor —
                        // replacing the deleted magic `TypeName == "target"`
                        // literal substitution. Bound by position (the handler's
                        // first parameter), not a fixed `"target"` spelling.
                        // Pass-2 (`process_comptime_directives`) resolves
                        // identically (same handler, same owner), so both phases
                        // reserve one expansion identity per generated method.
                        let owner =
                            TargetOwner::new(disc_target.name(), disc_target.nominal_shape());
                        Self::resolve_extend_owner_placeholder(
                            &mut execution.directives,
                            &owner,
                            handler.params.first().map(|p| p.name.as_str()),
                        );

                        for directive in execution.directives {
                            // ADR-009 E2 #18 (slice 3): pre-analysis
                            // materialization of a const-free FUNCTION-target
                            // `replace body` edit. Handled BEFORE the item-
                            // producing match below because a replacement is a
                            // BODY EDIT of an existing function, not a new item.
                            // The build runs inside the already-open C2
                            // `InstallTransaction`, so the shadow reservation is
                            // journaled; the driver applies the resulting edit to
                            // the analysis-program clone before the analyzer runs,
                            // which is what publishes the replacement closures'
                            // structural facts and flips C0911. Pass-2 still does
                            // the authoritative body swap byte-unchanged.
                            //
                            // A NON-function target `replace body` is left to
                            // pass-2 to reject ("only valid when compiling function
                            // targets"); a CONST-template target stays a pass-2
                            // concern (slice-0 §"Scoping boundary surfaced": its
                            // body may depend on a per-call-site const
                            // specialization absent from this single-program
                            // pre-analysis view).
                            if let super::comptime_builtins::ComptimeDirective::ReplaceBody {
                                body,
                            } = &directive
                            {
                                if let Some(func_def) = disc_target.function_def() {
                                    let const_free =
                                        !func_def.params.iter().any(|p| p.is_const);
                                    // Slice-3 scope: top-level function targets only.
                                    // A module-NESTED function target would need
                                    // module-path-aware targeting of the analysis
                                    // clone (and module-target pre-analysis is the
                                    // deferred E2-D9 territory), so a nested edit
                                    // stays a pass-2 concern for now — no regression
                                    // (its C0911 quarantine is the pre-existing
                                    // state), just not-yet-materialized.
                                    let top_level =
                                        disc_target.lexical_module_path().is_none();
                                    // Materialize ONLY closure-bearing replacements:
                                    // pre-analysis materialization exists to publish
                                    // a replacement closure's structural inference
                                    // fact (the C0911 flip). A closure-FREE
                                    // replacement has no such fact, so materializing
                                    // it would only add analyzer exposure with no
                                    // benefit — those edits stay pass-2-only,
                                    // byte-identical to the legacy behavior (the
                                    // "legacy route untouched" control). Same walk
                                    // the stamping uses, so detection and stamping
                                    // agree.
                                    let carries_closure = !shape_ast::transform::
                                        generated_closure_source_paths(body, &[])
                                        .is_empty();
                                    // Record ONE edit per target: a second
                                    // `replace body` on the same function is the
                                    // pass-2 "multiple `replace body` … ambiguous"
                                    // rejection — materializing both would instead
                                    // surface a duplicate-function analysis error
                                    // and mask that authoritative message. First
                                    // edit wins here; pass-2 emits the rejection.
                                    let already_edited = self
                                        .pending_replace_body_analysis
                                        .iter()
                                        .any(|edit| edit.target_name() == func_def.name);
                                    if const_free
                                        && top_level
                                        && carries_closure
                                        && !already_edited
                                    {
                                        let checked = self.build_checked_replace_body(
                                            func_def,
                                            body,
                                            &expansion_site,
                                        )?;
                                        self.pending_replace_body_analysis.push(checked);
                                    }
                                }
                                continue;
                            }
                            // ADR-009 E3 (slice S1): the executed pre-pass is
                            // now the SINGLE authority for BOTH generated
                            // directive shapes — the computed
                            // `extend (expr)` snippet (`ExtendItems`) AND the
                            // direct `extend target { method }` handler form
                            // (`Extend`). The deleted non-evaluating static AST
                            // scan formerly carried the direct form into the
                            // analysis program; here the direct extend is
                            // target-substituted and normalized to the same
                            // `Item::Extend` the `ExtendItems` path emits, so a
                            // single item-processing loop reserves method
                            // signatures and returns the block for the
                            // analyzer. Pass-2's `apply_comptime_extend`
                            // re-issues the identical reservation (same
                            // `annotation_expansion_site`) and compiles the
                            // bodies.
                            let items: Vec<Item> = match directive {
                                super::comptime_builtins::ComptimeDirective::ExtendItems {
                                    items,
                                } => items,
                                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                                    // ADR-009 E3 (S4, U11): the `extend <target>`
                                    // OWNER placeholder was already resolved above
                                    // by `resolve_extend_owner_placeholder`
                                    // (position-0 binding against the typed
                                    // `TargetOwner`) — matching pass-2 exactly, so
                                    // both phases produce the identical
                                    // `Item::Extend` and reserve one identity. The
                                    // deleted magic `TypeName == "target"` literal
                                    // substitution formerly lived here.
                                    vec![Item::Extend(extend, expansion_site.application_span())]
                                }
                                // ADR-009 C3 #14 (slice 2, S2b): documented
                                // PRE-PASS no-op — an `install(...)` directive
                                // applies at the authoritative pass-2 consumer
                                // only (never double-install; a non-function
                                // target's install is ALSO pass-2's named
                                // rejection, `process_comptime_directives`).
                                super::comptime_builtins::ComptimeDirective::InstallHookTemplate {
                                    ..
                                } => continue,
                                _ => continue,
                            };
                            // ADR-009 D1 (S2), rejection row 1: generated decls
                            // must anchor at the real application span; the
                            // generator-definition anchor is held to the same
                            // rule (S4).
                            let source_anchor =
                                expansion_site.source_anchor().map_err(|message| {
                                    self.expansion_rejection(message, &expansion_site)
                                })?;
                            let generator_anchor =
                                expansion_site.generator_anchor().map_err(|message| {
                                    self.expansion_rejection(message, &expansion_site)
                                })?;
                            for item in items {
                                match item {
                                    Item::Function(mut func_def, _span) => {
                                        let content = generated_free_fn_content(&func_def);
                                        // ADR-009 D1 (S3): anchor AFTER the raw
                                        // content fingerprint, so pass-2's raw
                                        // hash of the same output agrees.
                                        anchor_generated_function_decl(
                                            &mut func_def,
                                            expansion_site.application_span(),
                                        );
                                        let node_path = GeneratedNodePath::decl_root(format!(
                                            "fn:{}",
                                            func_def.name
                                        ));
                                        let origin = GeneratedOrigin {
                                            expansion: expansion_site.identity().clone(),
                                            node_path: node_path.clone(),
                                            source_anchor,
                                        };
                                        self.stamp_generated_closure_provenance(
                                            &mut func_def.body,
                                            &origin,
                                            &func_def.name,
                                        );
                                        match self.reserve_generated_decl_journaled(
                                            &func_def.name,
                                            origin,
                                            content,
                                            generator_anchor,
                                        ) {
                                            Ok(SymbolReservation::Fresh(_)) => {
                                                // Register the signature NOW so the
                                                // analyzer, function-registration pass, and
                                                // every user body (`fn main`) can resolve
                                                // the call. The BODY is still compiled by
                                                // pass-2's `apply_comptime_extend_items`
                                                // (`compile_function`) when the
                                                // annotated type compiles — the identical
                                                // path as before this pre-pass, so the
                                                // generated function's runtime/JIT
                                                // characteristics are unchanged.
                                                //
                                                // ADR-009 D1 (S4), row 7:
                                                // registration failures on the
                                                // generated decl carry full
                                                // expansion provenance.
                                                self.register_function(&func_def).map_err(|e| {
                                                    self.build_generated_decl_failure(
                                                        &e,
                                                        &func_def.name,
                                                        &node_path,
                                                        &expansion_site,
                                                    )
                                                })?;
                                                // ADR-009 D2: the discovered header
                                                // is immutable through the fixed
                                                // point (DISCOVERY_HEADER_MUTATED on
                                                // a differing re-derivation).
                                                discovery
                                                    .record_header(&func_def.name, content)
                                                    .map_err(|message| {
                                                        self.build_discovery_failure(
                                                            message,
                                                            Some(&expansion_site),
                                                        )
                                                    })?;
                                                generated.push(Item::Function(
                                                    func_def,
                                                    expansion_site.application_span(),
                                                ));
                                            }
                                            Ok(SymbolReservation::Reissued(_)) => {}
                                            Err(message) => {
                                                return Err(self.expansion_rejection(
                                                    message,
                                                    &expansion_site,
                                                ));
                                            }
                                        }
                                    }
                                    Item::Extend(mut extend, _span) => {
                                        // §4.9.1: a comptime-emitted type-extension
                                        // method (`u.to_json()`) must be visible to
                                        // the analyzer, method-dispatch resolution, and
                                        // every user body BEFORE pass-2 — exactly like
                                        // a generated free function. Reserve each
                                        // method's identity and register its SIGNATURE
                                        // now (keyed by its desugared `Type.method`
                                        // name), and return the `extend` block so the
                                        // analyzer learns the method on the type.
                                        // Pass-2's `apply_comptime_extend` re-issues the
                                        // same reservation and fills each pre-registered
                                        // slot through the normal function driver, so
                                        // generated methods get the same MIR/JIT surface
                                        // as hand-written `extend` methods.
                                        let extend_type_str = match &extend.type_name {
                                            shape_ast::ast::TypeName::Simple(name) => name.clone(),
                                            shape_ast::ast::TypeName::Generic { name, .. } => {
                                                name.clone()
                                            }
                                        };
                                        let mut any_new = false;
                                        for method in &extend.methods {
                                            let content = generated_extend_method_content(
                                                &extend.type_name,
                                                method,
                                            );
                                            let mut func_def = self
                                                .desugar_extend_method(method, &extend.type_name)?;
                                            // ADR-009 D1 (S3): anchor AFTER the
                                            // raw content fingerprint (pass-2
                                            // hashes the same raw AST).
                                            anchor_generated_function_decl(
                                                &mut func_def,
                                                expansion_site.application_span(),
                                            );
                                            let node_path = GeneratedNodePath::decl_root(format!(
                                                "extend:{extend_type_str}"
                                            ))
                                            .child(format!("method:{}", method.name));
                                            let origin = GeneratedOrigin {
                                                expansion: expansion_site.identity().clone(),
                                                node_path: node_path.clone(),
                                                source_anchor,
                                            };
                                            let owner =
                                                format!("{extend_type_str}.{}", method.name);
                                            self.stamp_generated_closure_provenance(
                                                &mut func_def.body,
                                                &origin,
                                                &owner,
                                            );
                                            match self.reserve_generated_decl_journaled(
                                                &func_def.name,
                                                origin,
                                                content,
                                                generator_anchor,
                                            ) {
                                                Ok(SymbolReservation::Fresh(_)) => {
                                                    // ADR-009 D1 (S4), row 7:
                                                    // provenance on registration
                                                    // failures.
                                                    self.register_function(&func_def).map_err(
                                                        |e| {
                                                            self.build_generated_decl_failure(
                                                                &e,
                                                                &func_def.name,
                                                                &node_path,
                                                                &expansion_site,
                                                            )
                                                        },
                                                    )?;
                                                    // ADR-009 D2: header immutable
                                                    // through the fixed point.
                                                    discovery
                                                        .record_header(&func_def.name, content)
                                                        .map_err(|message| {
                                                            self.build_discovery_failure(
                                                                message,
                                                                Some(&expansion_site),
                                                            )
                                                        })?;
                                                    any_new = true;
                                                }
                                                Ok(SymbolReservation::Reissued(_)) => {}
                                                Err(message) => {
                                                    return Err(self.expansion_rejection(
                                                        message,
                                                        &expansion_site,
                                                    ));
                                                }
                                            }
                                        }
                                        if any_new {
                                            // ADR-009 D1 (S3): the analysis copy
                                            // anchors its decl-level spans at the
                                            // application site too (method body
                                            // spans stay handler-emitted — D2
                                            // scope line, see
                                            // `anchor_generated_function_decl`).
                                            for method in &mut extend.methods {
                                                method.span = expansion_site.application_span();
                                                self.stamp_generated_analysis_method(
                                                    method,
                                                    &expansion_site,
                                                    source_anchor,
                                                    &extend_type_str,
                                                );
                                            }
                                            generated.push(Item::Extend(
                                                extend,
                                                expansion_site.application_span(),
                                            ));
                                        }
                                    }
                                    Item::StructType(sd, _span) => {
                                        // ADR-009 D2 additions-only re-scan: a
                                        // generated ANNOTATED type is enqueued for
                                        // the next discovery round so its own
                                        // annotation applications are discovered
                                        // (its header stays immutable once
                                        // discovered). The producing application is
                                        // recorded as the output-triggers edge
                                        // source for cycle detection. The v1
                                        // directive surface never emits a generated
                                        // annotated type, so this arm is dormant on
                                        // real programs — it makes multi-level
                                        // discovery total.
                                        if !sd.annotations.is_empty()
                                            && known_type_symbols.insert(sd.name.clone())
                                        {
                                            type_producer.insert(
                                                sd.name.clone(),
                                                expansion_site.identity().clone(),
                                            );
                                            newly_generated_types.push(
                                                DeclarationDiscoveryTarget::Struct {
                                                    definition: sd,
                                                    lexical_module_path: disc_target
                                                        .lexical_module_path()
                                                        .map(str::to_string),
                                                },
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            // ADR-009 D2 additions-only: enqueue this round's newly generated
            // annotated types for re-scan in the next discovery round.
            for sd in newly_generated_types {
                worklist.push(sd);
            }
        }

        // ADR-009 D2: the fixed point converged — the worklist is empty and
        // every reserved generated identity was defined
        // (RESERVED_IDENTITY_UNDEFINED otherwise). Declaration discovery is
        // complete BEFORE the analyzer/inference/body-checking runs below
        // (discovery-before-body ordering, Decision 67).
        discovery
            .converge()
            .map_err(|message| self.build_discovery_failure(message, None))?;

        // The generated free functions are returned so the driver can add them
        // to the ANALYSIS program (the analyzer type-checks their call sites and
        // their bodies). They are NOT added to the compiled program's items:
        // their signatures are already registered above, and their bodies are
        // compiled by pass-2 through the normal function driver.
        Ok(generated)
    }

    pub(super) fn apply_comptime_extend_items(
        &mut self,
        items: Vec<shape_ast::ast::Item>,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<()> {
        use shape_ast::ast::Item;

        let mut functions: Vec<FunctionDef> = Vec::new();
        let mut extends: Vec<shape_ast::ast::ExtendStatement> = Vec::new();
        for item in items {
            match item {
                Item::Function(func_def, _) => functions.push(func_def),
                Item::Extend(extend, _) => extends.push(extend),
                other => {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "comptime `extend (...)` generated an item kind that is not \
                             supported in v1 — only free functions and `extend Type {{ ... }}` \
                             blocks may be generated (got {})",
                            Self::generated_item_kind_name(&other)
                        ),
                        location: None,
                    });
                }
            }
        }

        // ADR-009 D1 (S2), rejection row 1: generated decls must anchor at
        // the real application span — refused before any registration. The
        // generator-definition anchor is held to the same rule (S4).
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;

        // comptime-excellence §4.5.1 + D1 identity-keyed dedup: if the
        // whole-program pre-pass already reserved this generated free
        // function's identity and registered its SIGNATURE (so it is visible
        // to `fn main()` and every user body), the reservation is re-issued —
        // skip re-registering it here (a second `register_function` would
        // create a duplicate slot). The body is still compiled below via
        // `compile_function`, which fills the pre-registered slot, so the
        // generated function is compiled exactly once through the identical
        // path as before the pre-pass existed.
        for func_def in &mut functions {
            let content = generated_free_fn_content(func_def);
            // ADR-009 D1 (S3): anchor AFTER the raw content fingerprint —
            // the pre-pass hashed the same raw AST (see
            // `anchor_generated_function_decl`).
            anchor_generated_function_decl(func_def, site.application_span());
            let node_path = GeneratedNodePath::decl_root(format!("fn:{}", func_def.name));
            let origin = GeneratedOrigin {
                expansion: site.identity().clone(),
                node_path: node_path.clone(),
                source_anchor,
            };
            self.stamp_generated_closure_provenance(&mut func_def.body, &origin, &func_def.name);
            match self.reserve_generated_decl_journaled(
                &func_def.name,
                origin,
                content,
                generator_anchor,
            ) {
                Ok(SymbolReservation::Fresh(_)) => {
                    // ADR-009 D1 (S4), row 7: provenance on registration
                    // failures.
                    self.register_function(func_def).map_err(|e| {
                        self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                    })?;
                }
                Ok(SymbolReservation::Reissued(_)) => {}
                Err(message) => return Err(self.expansion_rejection(message, site)),
            }
        }
        for func_def in &functions {
            // WF-3D generated-fn JIT parity: compile via the FULL driver
            // (`compile_function`), not the bytecode-only `compile_function_body`.
            // The driver lowers the body to MIR and attaches `Function.mir_data`,
            // which the JIT's Phase-4 MirToIR pass requires — a bytecode-only
            // generated function fails Phase-4 ("has no MIR data") and forces a
            // whole-program deopt to the interpreter. A hand-written free function
            // goes through `compile_function`; routing the generated one through
            // the same path gives it native JIT codegen and VM == JIT.
            //
            // ADR-009 C2 #13 (slice 2, battery row 10b): the D6 conservative
            // async-drop-context guard, on the AUTHENTICATED generated free
            // function just before its pass-2 compile — same install-span
            // rollback guarantee as the extend-method site. The origin is
            // rebuilt here exactly as the registration loop above minted it
            // (`site` + `fn:<name>`); the issuer capability, not the path,
            // authenticates it.
            let node_path = GeneratedNodePath::decl_root(format!("fn:{}", func_def.name));
            let origin = GeneratedOrigin {
                expansion: site.identity().clone(),
                node_path: node_path.clone(),
                source_anchor,
            };
            // ADR-009 C2 #13 (slice 2, D6; slice 4 threading): arm the
            // async-drop-context gate for this generated free function by
            // passing its node-borne provenance as a parameter (no shared
            // field); the gate runs at the end of the inner compile
            // (emission-authority drop signal + suspension scan on the
            // authenticated body).
            let node_origin = origin.to_node_origin(&self.generated_node_issuer, &func_def.name);
            // ADR-009 D1 (S4), rejection row 7: a body error inside the
            // generated free function surfaces with full expansion
            // provenance (generated-node + application + generator
            // locations).
            self.compile_function_with_generated_origin(func_def, Some(node_origin))
                .map_err(|e| {
                    self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                })?;
        }
        for extend in extends {
            self.apply_comptime_extend(extend, target_name, site)?;
        }
        Ok(())
    }

    fn generated_item_kind_name(item: &shape_ast::ast::Item) -> &'static str {
        use shape_ast::ast::Item;
        match item {
            Item::Function(..) => "function",
            Item::Extend(..) => "extend",
            Item::StructType(..) => "type",
            Item::Enum(..) => "enum",
            Item::Trait(..) => "trait",
            Item::Impl(..) => "impl",
            _ => "item",
        }
    }

    fn annotate_comptime_extend_method_params(
        &self,
        methods: &mut [shape_ast::ast::types::MethodDef],
        target_name: &str,
    ) {
        let Some(struct_def) = self.comptime_context_struct_defs.get(target_name) else {
            return;
        };
        let field_types: HashMap<&str, &TypeAnnotation> = struct_def
            .fields
            .iter()
            .map(|field| (field.name.as_str(), &field.type_annotation))
            .collect();
        let target_annotation = TypeAnnotation::Basic(target_name.to_string());

        for method in methods {
            for param_idx in 0..method.params.len() {
                if method.params[param_idx].type_annotation.is_some() {
                    continue;
                }
                let Some(param_name) = method.params[param_idx].simple_name() else {
                    continue;
                };

                let inferred = if Self::body_accesses_target_field_on_param(
                    &method.body,
                    param_name,
                    &field_types,
                ) {
                    Some(target_annotation.clone())
                } else {
                    Self::infer_param_type_from_self_field_binary(
                        &method.body,
                        param_name,
                        &field_types,
                    )
                };

                if let Some(type_annotation) = inferred {
                    method.params[param_idx].type_annotation = Some(type_annotation);
                }
            }
        }
    }

    fn body_accesses_target_field_on_param(
        body: &[Statement],
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        body.iter().any(|stmt| {
            Self::statement_accesses_target_field_on_param(stmt, param_name, field_types)
        })
    }

    fn statement_accesses_target_field_on_param(
        stmt: &Statement,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }
            Statement::VariableDecl(decl, _) => decl.value.as_ref().is_some_and(|expr| {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }),
            Statement::Assignment(assign, _) => {
                Self::expr_accesses_target_field_on_param(&assign.value, param_name, field_types)
            }
            Statement::If(if_stmt, _) => {
                Self::expr_accesses_target_field_on_param(
                    &if_stmt.condition,
                    param_name,
                    field_types,
                ) || if_stmt.then_body.iter().any(|stmt| {
                    Self::statement_accesses_target_field_on_param(stmt, param_name, field_types)
                }) || if_stmt.else_body.as_ref().is_some_and(|body| {
                    body.iter().any(|stmt| {
                        Self::statement_accesses_target_field_on_param(
                            stmt,
                            param_name,
                            field_types,
                        )
                    })
                })
            }
            _ => false,
        }
    }

    fn expr_accesses_target_field_on_param(
        expr: &Expr,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        match expr {
            Expr::PropertyAccess { object, .. } => {
                matches!(&**object, Expr::Identifier(name, _) if name == param_name)
                    || Self::expr_accesses_target_field_on_param(object, param_name, field_types)
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::expr_accesses_target_field_on_param(left, param_name, field_types)
                    || Self::expr_accesses_target_field_on_param(right, param_name, field_types)
            }
            Expr::UnaryOp { operand, .. } => {
                Self::expr_accesses_target_field_on_param(operand, param_name, field_types)
            }
            Expr::FunctionCall { args, .. }
            | Expr::QualifiedFunctionCall { args, .. }
            | Expr::Array(args, _) => args.iter().any(|expr| {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }),
            _ => false,
        }
    }

    fn infer_param_type_from_self_field_binary(
        body: &[Statement],
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        body.iter().find_map(|stmt| {
            Self::statement_infer_param_type_from_self_field_binary(stmt, param_name, field_types)
        })
    }

    fn statement_infer_param_type_from_self_field_binary(
        stmt: &Statement,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }
            Statement::VariableDecl(decl, _) => decl.value.as_ref().and_then(|expr| {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }),
            Statement::Assignment(assign, _) => Self::expr_infer_param_type_from_self_field_binary(
                &assign.value,
                param_name,
                field_types,
            ),
            Statement::If(if_stmt, _) => Self::expr_infer_param_type_from_self_field_binary(
                &if_stmt.condition,
                param_name,
                field_types,
            )
            .or_else(|| {
                if_stmt.then_body.iter().find_map(|stmt| {
                    Self::statement_infer_param_type_from_self_field_binary(
                        stmt,
                        param_name,
                        field_types,
                    )
                })
            })
            .or_else(|| {
                if_stmt.else_body.as_ref().and_then(|body| {
                    body.iter().find_map(|stmt| {
                        Self::statement_infer_param_type_from_self_field_binary(
                            stmt,
                            param_name,
                            field_types,
                        )
                    })
                })
            }),
            _ => None,
        }
    }

    fn expr_infer_param_type_from_self_field_binary(
        expr: &Expr,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        match expr {
            Expr::BinaryOp { left, right, .. } => {
                if Self::expr_is_identifier(right, param_name) {
                    if let Some(field_type) = Self::self_field_type(left, field_types) {
                        return Some(field_type.clone());
                    }
                }
                if Self::expr_is_identifier(left, param_name) {
                    if let Some(field_type) = Self::self_field_type(right, field_types) {
                        return Some(field_type.clone());
                    }
                }
                Self::expr_infer_param_type_from_self_field_binary(left, param_name, field_types)
                    .or_else(|| {
                        Self::expr_infer_param_type_from_self_field_binary(
                            right,
                            param_name,
                            field_types,
                        )
                    })
            }
            Expr::UnaryOp { operand, .. } => {
                Self::expr_infer_param_type_from_self_field_binary(operand, param_name, field_types)
            }
            Expr::FunctionCall { args, .. }
            | Expr::QualifiedFunctionCall { args, .. }
            | Expr::Array(args, _) => args.iter().find_map(|expr| {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }),
            _ => None,
        }
    }

    fn self_field_type<'a>(
        expr: &Expr,
        field_types: &HashMap<&str, &'a TypeAnnotation>,
    ) -> Option<&'a TypeAnnotation> {
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return None;
        };
        if matches!(&**object, Expr::Identifier(name, _) if name == "self") {
            field_types.get(property.as_str()).copied()
        } else {
            None
        }
    }

    fn expr_is_identifier(expr: &Expr, expected: &str) -> bool {
        matches!(expr, Expr::Identifier(name, _) if name == expected)
    }

    /// ADR-009 E2 #18 (slice 1/2): the SHARED per-item check sequence — anchor,
    /// generated closure-provenance stamp (`GeneratedNodeIssuer`), hygienic
    /// export reservation (`SymbolId`, journaled through the InstallTransaction)
    /// — used by BOTH the typed replace-module consumer (`build_checked_module`,
    /// slice 1) and the typed extend-items / `CheckedItem` consumer (slice 2),
    /// so the two never duplicate the sequence. Returns the stamped item and its
    /// reserved hygienic export symbol; the fingerprint is taken BEFORE anchoring
    /// so pass-2's raw hash agrees (ADR-009 D1 S3). Does NOT `register_function`:
    /// the caller decides (module-replace does not — the module-compile flow
    /// registers; extend-items does — see `apply_comptime_extend_items`).
    ///
    /// (Lives in `functions_annotations` rather than `comptime_fragments/`
    /// because `stamp_generated_closure_provenance` / `generated_free_fn_content`
    /// / `expansion_rejection` are module-private here; the DATA carriers
    /// `CheckedModule` / `CheckedItem` are the `comptime_fragments/` half.)
    fn check_generated_function_item(
        &mut self,
        mut func_def: FunctionDef,
        span: Span,
        site: &ExpansionSite,
        source_anchor: SourceAnchor,
        generator_anchor: SourceAnchor,
        node_path_prefix: &str,
    ) -> Result<(shape_ast::ast::Item, SymbolId)> {
        // Fingerprint BEFORE anchoring so pass-2's raw hash agrees (ADR-009 D1
        // S3), then re-base decl spans to the anchor.
        let content = generated_free_fn_content(&func_def);
        anchor_generated_function_decl(&mut func_def, site.application_span());
        let node_path =
            GeneratedNodePath::decl_root(format!("{node_path_prefix}:{}", func_def.name));
        let origin = GeneratedOrigin {
            expansion: site.identity().clone(),
            node_path,
            source_anchor,
        };
        self.stamp_generated_closure_provenance(&mut func_def.body, &origin, &func_def.name);
        let export = match self.reserve_generated_decl_journaled(
            &func_def.name,
            origin,
            content,
            generator_anchor,
        ) {
            Ok(SymbolReservation::Fresh(id)) | Ok(SymbolReservation::Reissued(id)) => id,
            Err(message) => return Err(self.expansion_rejection(message, site)),
        };
        Ok((shape_ast::ast::Item::Function(func_def, span), export))
    }

    /// ADR-009 E2 #18 (slice 1): build the typed `CheckedModule` for a
    /// `ReplaceModuleChecked` directive (the module-target consumer,
    /// `process_comptime_directives_for_module`). Each generated item is
    /// anchored, stamped with generated closure provenance
    /// (`stamp_generated_closure_provenance` / `GeneratedNodeIssuer`), and its
    /// declaration reserves a hygienic export symbol (`SymbolId`) — the SAME
    /// per-item sequence the fresh-generated declaration-discovery pre-pass runs
    /// in `materialize_computed_comptime_extends`, MINUS `register_function`
    /// (the module-compile flow qualifies + registers the replacement items
    /// itself; double-registration would collide). No source/JSON string
    /// participates. The reservation is JOURNALED, so a failed install rolls it
    /// back with the rest of the transaction.
    ///
    /// Non-function items (none are producible by the slice-1 typed producer
    /// `item_fn`, which mints only a single function) pass through unstamped —
    /// the typed producer's reach grows with the fragment schema, not here.
    pub(super) fn build_checked_module(
        &mut self,
        items: Vec<shape_ast::ast::Item>,
        site: &ExpansionSite,
    ) -> Result<super::comptime_fragments::CheckedModule> {
        use shape_ast::ast::Item;

        // Validated once and reused across items (both anchors are `Copy`), the
        // same way the discovery pre-pass reuses them across a round's decls.
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;

        let mut stamped: Vec<Item> = Vec::with_capacity(items.len());
        let mut exports = Vec::new();
        for item in items {
            match item {
                Item::Function(func_def, span) => {
                    let (checked, export) = self.check_generated_function_item(
                        func_def,
                        span,
                        site,
                        source_anchor,
                        generator_anchor,
                        "module_fn",
                    )?;
                    exports.push(export);
                    stamped.push(checked);
                }
                other => stamped.push(other),
            }
        }
        Ok(super::comptime_fragments::CheckedModule::new(stamped, exports))
    }

    /// ADR-009 E2 #18 (slice 3): build the typed `CheckedReplaceBody` for a
    /// const-free FUNCTION-target `replace body` edit, so the driver can
    /// materialize the replacement PRE-ANALYSIS (swap the target's body + prepend
    /// the hygienic `ctx.original` shadow into the analysis-program clone) before
    /// `analyze_program_full`. This is what flips the C0911 quarantine: the
    /// analyzer then infers the STAMPED replacement closures and publishes their
    /// structural specialization facts, keyed by the same content-derived
    /// closure-origin identity pass-2's capture descriptor uses (both stamp with
    /// the SAME `ExpansionSite` via `stamp_generated_replacement_body`).
    ///
    /// The sequence mirrors pass-2's `ReplaceBody` application
    /// (`process_comptime_directives_for_function`) exactly — same shadow name,
    /// same `ctx.original` capability + rewrite, same stamp — MINUS the
    /// authoritative install (pass-2 still swaps the shipped body and registers
    /// the shadow byte-unchanged). The ONE persistent publication here is the
    /// shadow's reserved hygienic identity, JOURNALED through the already-open C2
    /// `InstallTransaction`, so a failed compile rolls it back atomically. The
    /// shadow's own closures are deliberately NOT generated-stamped — it retains
    /// user code (the capture gate follows the node stamp, not the reservation).
    ///
    /// Scoping (slice-0 §"Scoping boundary surfaced"): the caller restricts this
    /// to const-free targets. A const-template `replace body` whose emitted body
    /// depends on a per-call-site const specialization would materialize
    /// differently (or not at all) in this single-program pre-analysis view, so
    /// those edits stay a pass-2 concern.
    pub(super) fn build_checked_replace_body(
        &mut self,
        target: &FunctionDef,
        replacement_body: &[Statement],
        site: &ExpansionSite,
    ) -> Result<super::comptime_fragments::CheckedReplaceBody> {
        // Same hygienic shadow identity + `ctx.original` capability pass-2 builds
        // (`original_body_shadow_name` is a stable digest of the function name).
        let shadow_name = self.original_body_shadow_name(&target.name);
        let capability = self.build_original_capability(target, shadow_name.clone())?;

        // Rewrite every `ctx.original(args)` in the replacement into a direct
        // typed call to the hygienic shadow, seeding scope with the target's
        // parameter binders (incl. destructuring) and `self` — identical to the
        // pass-2 rewrite so the analyzed body matches the shipped one.
        let mut bound_receivers: HashSet<String> = target
            .params
            .iter()
            .flat_map(|p| p.get_identifiers())
            .collect();
        bound_receivers.insert("self".to_string());
        let rewritten = super::original_body_rewrite::rewrite_original_calls_in_statements(
            replacement_body,
            &bound_receivers,
            capability.shadow_name(),
        );

        // Stamp the replacement's closures with generated provenance through the
        // SAME `stamp_generated_replacement_body(site)` pass-2 calls — so the
        // analyzer's published fact and pass-2's capture descriptor share one
        // content-derived closure-origin identity. The annotations are dropped on
        // the analysis copy (it is analyzed, never re-discovered).
        let mut replacement = target.clone();
        replacement.annotations = Vec::new();
        replacement.body = rewritten;
        let _replacement_origin = self.stamp_generated_replacement_body(&mut replacement, site)?;

        // The hygienic shadow: the pre-annotation body under the shadow name (the
        // `PendingOriginalBodyShadow::new` emission shape). NOT closure-stamped.
        let shadow = FunctionDef {
            name: shadow_name.clone(),
            name_span: target.name_span,
            declaring_module_path: target.declaring_module_path.clone(),
            doc_comment: None,
            params: target.params.clone(),
            return_type: target.return_type.clone(),
            body: target.body.clone(),
            type_params: target.type_params.clone(),
            annotations: Vec::new(),
            where_clause: target.where_clause.clone(),
            is_async: target.is_async,
            is_comptime: target.is_comptime,
        };

        // Reserve the shadow's hygienic identity, JOURNALED through the open
        // transaction (rolls back on a failed install — the atomicity pin). The
        // node path distinguishes the shadow from the replacement stamp above.
        let content = generated_free_fn_content(&shadow);
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let origin = GeneratedOrigin {
            expansion: site.identity().clone(),
            node_path: GeneratedNodePath::decl_root(format!("shadow_fn:{}", target.name)),
            source_anchor,
        };
        let shadow_export = match self.reserve_generated_decl_journaled(
            &shadow_name,
            origin,
            content,
            generator_anchor,
        ) {
            Ok(SymbolReservation::Fresh(id)) | Ok(SymbolReservation::Reissued(id)) => id,
            Err(message) => return Err(self.expansion_rejection(message, site)),
        };

        Ok(super::comptime_fragments::CheckedReplaceBody::new(
            target.name.clone(),
            replacement.body,
            shadow,
            shadow_export,
        ))
    }

    pub(super) fn process_comptime_directives(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<bool> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::ExtendItems { items } => {
                    self.apply_comptime_extend_items(items, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType { .. }
                | super::comptime_builtins::ComptimeDirective::SetParamValue { .. } => {
                    return Err(Self::directive_error(
                        "`set param` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {
                    return Err(Self::directive_error(
                        "`set return` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { .. } => {
                    return Err(Self::directive_error(
                        "`replace body` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModuleChecked { .. } => {
                    return Err(Self::directive_error(
                        "`replace module` directives are only valid when compiling module targets",
                    ));
                }
                // ADR-009 C3 #14 (slice 2, S2b): a hook template installs
                // onto a FUNCTION's before/after seam; this consumer compiles
                // type targets — named rejection with the positive twin.
                super::comptime_builtins::ComptimeDirective::InstallHookTemplate { .. } => {
                    return Err(Self::directive_error(
                        "`install` directives are only valid when compiling function targets \
                         (a hook template attaches to a function's before/after seam); apply \
                         the installing annotation to a function",
                    ));
                }
            }
        }
        Ok(removed)
    }

    /// ADR-009 E3 (S3, U11): the unspellable HYGIENIC registry name of the
    /// `replace body` shadow that holds `func_name`'s pre-annotation body. The
    /// nonce is a stable digest of `func_name`, so the shadow re-registers
    /// idempotently (one shadow per annotated function — `register_function`
    /// dedups by name) instead of minting a fresh orphan on every recompile.
    fn original_body_shadow_name(&self, func_name: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        func_name.hash(&mut hasher);
        self.mint_hygienic_fn_name_stable(HygienicRole::OriginalBodyShadow, hasher.finish())
    }

    /// ADR-009 E3 (S3, U11): build the typed `ctx.original` capability. The
    /// pre-annotation body's signature IS the annotated function's signature
    /// (the shadow clones params + return), so its `FrozenCallable` (B6)
    /// identity is canonicalized through THE single per-compilation-unit
    /// semantic-freeze handle. Reaching this without an installed freeze is the
    /// named `NO_FREEZE_HANDLE_DIAGNOSTIC` compile error (rejection-matrix
    /// row 3); a parameter with no resolvable type / an unfreezable return type
    /// surfaces the canonicalizer's named error (no partial descriptor, no
    /// string/Any capability surface).
    pub(super) fn build_original_capability(
        &self,
        func_def: &FunctionDef,
        shadow_name: String,
    ) -> Result<OriginalCapability> {
        let overlay = self.comptime_freeze_overlay()?;
        let callable = canonical_original_callable(overlay.as_ref(), func_def)
            .map_err(Self::directive_error)?;
        Ok(OriginalCapability {
            shadow_name,
            callable,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn process_comptime_directives_for_function(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
        func_def: &mut FunctionDef,
        site: &ExpansionSite,
        inferred_reference_optimizations: &[Option<ParamPassMode>],
        pending_original_body_shadow: &mut Option<PendingOriginalBodyShadow>,
        // ADR-009 C2 #13 (slice 4): set by the `replace body` arm to the
        // REPLACEMENT's node-borne provenance so the D6 gate authenticates the
        // swapped body (see `execute_comptime_handlers`).
        replacement_body_origin: &mut Option<GeneratedNodeOrigin>,
        // ADR-009 C3 #14 (slice 2, S2b): the installing annotation's name
        // (origin, parameter-threaded — never ambient) for the
        // `InstallHookTemplate` apply seam's attribution + registry row.
        annotation_name: &str,
        // ADR-009 C3 #14 (slice 2, S2b): the per-target hook-install
        // accumulator (the `pending_original_body_shadow` threading pattern —
        // a parameter owned by `execute_comptime_handlers`, never ambient
        // state). Installs ACCUMULATE across the target's annotations in
        // application order; the weave materializes ONCE after the last
        // handler (S2c — this stage stops at staged installs + registry).
        staged_hook_installs: &mut Vec<StagedHookInstall>,
    ) -> Result<bool> {
        let mut removed = false;
        // ADR-009 C3 #14 (fix-round-1): SNAPSHOT-resolve every install
        // handle BEFORE any directive applies — applying a directive below
        // (a polymorphic `specialize_template`'s nested `compile_function`,
        // an `ExtendItems` compile) can trigger a NESTED annotation-handler
        // run that clears + repopulates the per-run execute-populated
        // stores, so a LATER install's lazy index resolution would read
        // across store generations (miss → misdiagnosed internal error;
        // repopulated store → the WRONG template installed silently). Same
        // snapshot discipline `take_comptime_directives` applies to the
        // directive buffer, extended to the handles the directives carry.
        let mut resolved_install_templates =
            self.snapshot_install_hook_template_handles(&directives, site)?;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::ExtendItems { items } => {
                    self.apply_comptime_extend_items(items, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    *pending_original_body_shadow = None;
                    // A removed target installs no body, so discard any staged
                    // `replace body` provenance too (the D6 gate is skipped for
                    // removed functions, but keep the out-param consistent).
                    *replacement_body_origin = None;
                    // ADR-009 C3 #14 (slice 2, S2b): a removed target weaves
                    // nothing — drop its staged hook installs and their
                    // registry rows (a row for a never-woven install would
                    // misreport; the journaled length-undo stays correct).
                    staged_hook_installs.clear();
                    self.discard_hook_install_rows_for_target(&func_def.name);
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType {
                    param_name,
                    type_annotation,
                } => {
                    // E1-D4: ONE param-miss diagnostic everywhere — the install
                    // applier resolves through the same `resolve_param_id` as the
                    // analysis pre-pass, so a miss is `[C0930]` here too (no
                    // annotation/span context at this phase → `None`).
                    let param_id = param_selection::resolve_param_id(
                        func_def,
                        &param_name,
                        "set param type",
                        None,
                        None,
                    )?;
                    let param = &mut func_def.params[param_id.index()];
                    if let Some(existing) = &param.type_annotation {
                        if existing != &type_annotation {
                            return Err(Self::directive_error(format!(
                                "cannot override explicit type of parameter '{}'",
                                param_name
                            )));
                        }
                    } else {
                        param.type_annotation = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::SetParamValue {
                    param_name,
                    value,
                } => {
                    let param_id = param_selection::resolve_param_id(
                        func_def,
                        &param_name,
                        "set param value",
                        None,
                        None,
                    )?;
                    let default_value =
                        Self::scalar_default_expr_from_kinded_slot(&param_name, &value)
                            .map_err(Self::directive_error)?;
                    func_def.params[param_id.index()].default_value = Some(default_value);
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { type_annotation } => {
                    if let Some(existing) = &func_def.return_type {
                        if existing != &type_annotation {
                            return Err(Self::directive_error(
                                "cannot override explicit function return type annotation",
                            ));
                        }
                    } else {
                        func_def.return_type = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { body } => {
                    if pending_original_body_shadow.is_some() {
                        return Err(Self::directive_error(format!(
                            "multiple `replace body` directives for function '{}' are ambiguous",
                            func_def.name
                        )));
                    }
                    // ADR-009 E3 (S3, U11): the pre-annotation body becomes a
                    // staged compiler-issued HYGIENIC shadow, reachable ONLY
                    // through the typed `ctx.original` capability. Staging
                    // preserves the untouched body and its exact parameter
                    // provenance until every handler outcome is known.
                    let shadow_name = self.original_body_shadow_name(&func_def.name);
                    let effective_pass_modes = self.effective_function_like_pass_modes(
                        Some(&func_def.name),
                        &func_def.params,
                        Some(&func_def.body),
                    );
                    // Build the typed `ctx.original` capability (B6
                    // FrozenCallable via the single freeze handle — row 3
                    // NO_FREEZE_HANDLE_DIAGNOSTIC otherwise), then rewrite every
                    // `ctx.original(args)` in the replacement into a direct
                    // typed `FunctionCall` to the hygienic shadow. The rewrite
                    // runs BEFORE the swapped body reaches MIR lowering / the
                    // MIR-derived type-inference pass, so the pre-annotation
                    // call is fully typed everywhere downstream (the shadow
                    // carries the original's EXACT signature). No hidden
                    // binding is injected into user scope, and no name-encoded
                    // alias resolves the call — the role is bound by the
                    // `.original` capability member, not a global spelling.
                    let capability = self.build_original_capability(func_def, shadow_name)?;
                    // Seed the receiver-scope with every identifier the target's
                    // parameters bind — including destructuring params (`fn f({x,
                    // y}: P)`), whose binders `simple_name()` would drop. Body-local
                    // bindings are added lexically inside the rewrite itself.
                    let mut bound_receivers: std::collections::HashSet<String> = func_def
                        .params
                        .iter()
                        .flat_map(|p| p.get_identifiers())
                        .collect();
                    bound_receivers.insert("self".to_string());
                    let mut replacement = func_def.clone();
                    replacement.body =
                        super::original_body_rewrite::rewrite_original_calls_in_statements(
                            &body,
                            &bound_receivers,
                            capability.shadow_name(),
                        );

                    // ADR-009 C2 #13 (slice 4): stamp the replacement's closures
                    // AND capture its node-borne declaration origin, so the D6
                    // async-drop-context gate at the end of `compile_function_inner`
                    // authenticates the swapped body (which compiles under the
                    // user function name and so has no name-recoverable
                    // provenance).
                    let replacement_origin =
                        self.stamp_generated_replacement_body(&mut replacement, site)?;
                    // ADR-009 C2 #13 (slice 4, D7): assert the edit-transaction
                    // shape — one expansion identity ([C0924]) over a complete
                    // current environment ([C0925]). Defense-in-depth: both hold
                    // by construction here (the origin was minted from `site`),
                    // so this surfaces a named rejection only if a future refactor
                    // splits the identity or installs from a partial environment.
                    self.guard_edit_transaction_shape(&replacement_origin, site)?;
                    let pending = PendingOriginalBodyShadow::new(
                        func_def,
                        capability,
                        inferred_reference_optimizations,
                        &effective_pass_modes,
                    )?;
                    *func_def = replacement;
                    *pending_original_body_shadow = Some(pending);
                    *replacement_body_origin = Some(replacement_origin);
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModuleChecked { .. } => {
                    return Err(Self::directive_error(
                        "`replace module` directives are only valid when compiling module targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::InstallHookTemplate { .. } => {
                    // ADR-009 C3 #14 (slice 2, S2b): the AUTHORITATIVE apply
                    // seam — pop the SNAPSHOT-resolved handle (fix-round-1:
                    // resolved at loop entry, never a live store read here),
                    // run the G8/driver rejections, compose
                    // `specialize_template` with the ALREADY-OPEN C2 install
                    // transaction (`compile_in_place`,
                    // compiler_impl_reference_model.rs:1985-1996, opens the
                    // journal at :1986 BEFORE the inner driver this consumer
                    // runs inside — E1-D6b, never a second transaction), bind
                    // the capture plan, stage the install on the per-target
                    // accumulator, and write one journaled registry row. Full
                    // sequence + rationale:
                    // `template_specialization::install_registry` module docs.
                    let bound = resolved_install_templates.pop_front().ok_or_else(|| {
                        ShapeError::RuntimeError {
                            message: "internal error: InstallHookTemplate directive without a \
                                      snapshot-resolved template (the batch snapshot resolves \
                                      one per install directive at loop entry)"
                                .to_string(),
                            location: None,
                        }
                    })?;
                    self.apply_install_hook_template(
                        &bound,
                        annotation_name,
                        func_def,
                        site,
                        staged_hook_installs,
                    )?;
                }
            }
        }
        Ok(removed)
    }

    /// S3 (design §4.5): a comptime `set return` / `set param` directive
    /// changed `effective_def`'s signature. Re-run the ordinary whole-program
    /// type analysis with the mutated signature patched in, so the directive
    /// re-enters the SAME body-vs-signature checker the explicit-annotation
    /// path uses. This turns the `set return string` on `fn answer() { 42 }`
    /// segfault into an ordinary compile error.
    pub(super) fn recheck_directive_mutated_signature(
        &mut self,
        effective_def: &FunctionDef,
    ) -> Result<()> {
        // Record this function's post-directive signature so later re-analyses
        // (for sibling directive-mutated functions) observe it too.
        self.directive_signature_overrides.insert(
            effective_def.name.clone(),
            (
                effective_def.params.clone(),
                effective_def.return_type.clone(),
            ),
        );
        // Only the strict compiler path executes code, so only it can segfault.
        // In RecoverAll (LSP) mode best-effort analysis already ran and nothing
        // executes; re-running here would double-report diagnostics.
        if !matches!(
            self.type_diagnostic_mode,
            crate::compiler::TypeDiagnosticMode::Strict
        ) {
            return Ok(());
        }
        let Some(base_program) = self.directive_reanalysis_program.clone() else {
            return Ok(());
        };
        // Patch every known directive override into a clone of the analyzed
        // program. Only signature fields (return type + per-param type
        // annotation / default value) are patched; bodies are left as analyzed.
        let mut program = base_program;
        let mut patched_any = false;
        for item in &mut program.items {
            if let shape_ast::ast::Item::Function(func, _) = item
                && let Some((params, return_type)) =
                    self.directive_signature_overrides.get(&func.name)
            {
                func.return_type = return_type.clone();
                for (param, patched) in func.params.iter_mut().zip(params.iter()) {
                    param.type_annotation = patched.type_annotation.clone();
                    param.default_value = patched.default_value.clone();
                }
                patched_any = true;
            }
        }
        if !patched_any {
            return Ok(());
        }
        let known_bindings = self.directive_reanalysis_known_bindings.clone();
        let result = shape_runtime::type_system::analyze_program_with_mode_and_comptime_context(
            &program,
            self.source_text.as_deref(),
            None,
            Some(&known_bindings),
            shape_runtime::type_system::TypeAnalysisMode::FailFast,
            self.comptime_mode,
        );
        if let Err(errors) = result {
            return Err(self.directive_signature_type_error(&effective_def.name, errors));
        }
        Ok(())
    }

    /// Wrap the post-directive analysis failure with an attribution that names
    /// the comptime directive as the source of the incompatible signature.
    fn directive_signature_type_error(
        &self,
        func_name: &str,
        errors: Vec<shape_runtime::type_system::TypeErrorWithLocation>,
    ) -> ShapeError {
        let (detail, location) = match Self::type_errors_to_shape(errors) {
            ShapeError::SemanticError { message, location } => (message, location),
            other => (format!("{}", other), None),
        };
        ShapeError::SemanticError {
            message: format!(
                "a comptime directive set a return/parameter type on '{}' that its body does not satisfy: {}",
                func_name, detail
            ),
            location,
        }
    }

    /// Validate that all annotations on a function are allowed for function targets.
    pub(super) fn validate_annotation_targets(&self, func_def: &FunctionDef) -> Result<()> {
        self.check_duplicate_annotations(&func_def.annotations, func_def.name_span)?;
        for ann in &func_def.annotations {
            self.validate_annotation_target_usage(
                ann,
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                func_def.name_span,
            )?;
        }
        Ok(())
    }
}

// ADR-009 §4.1 (ticket A1, slice S3) — the annotation-handler freeze gate.
//
// Pre-pass freeze rule (S3, named rule per plan graft 4): the speculative
// annotation pre-passes (`materialize_computed_comptime_extends` and
// `apply_function_comptime_signature_directives_for_analysis`) consume the
// SAME registration-complete freeze handle as the authoritative pass-2
// execution — the freeze barrier runs BEFORE them in `compile()`. A pre-pass
// comptime site that cannot obtain the handle is the row-3 named compile
// error (`NO_FREEZE_HANDLE_DIAGNOSTIC`); exemption-by-suppression, empty
// snapshots and `Option<freeze>` are forbidden shapes. Dec 52 ordering: a
// freeze-boundary rejection fires at the barrier, BEFORE any handler body
// executes.
#[cfg(test)]
mod s3_freeze_gate_tests {
    use super::BytecodeCompiler;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Rejection-matrix row 3, type-target pre-pass: running the speculative
    /// extends pre-pass on a compiler whose freeze barrier has not run is a
    /// compile error with the named diagnostic — the pre-pass consumes the
    /// real handle, it does not fall back to a reflection-rejecting module.
    #[test]
    fn extends_prepass_without_freeze_handle_is_the_named_row3_compile_error() {
        let program = parse(
            r#"
annotation touch() on type {
  comptime post(target, ctx) {
    1
  }
}

@touch()
type Probe { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .materialize_computed_comptime_extends(&program)
            .expect_err("pre-barrier pre-pass site must be a compile error");
        assert!(
            error.to_string().contains("no semantic freeze handle"),
            "row-3 named diagnostic missing: {error}"
        );
    }

    /// Rejection-matrix row 3, function-target pre-pass (signature
    /// directives): same gate, same named diagnostic.
    #[test]
    fn signature_directive_prepass_without_freeze_handle_is_the_named_row3_compile_error() {
        let mut program = parse(
            r#"
annotation touch() on function {
  comptime post(target, ctx) {
    1
  }
}

@touch()
fn probe() -> int { 2 }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .apply_function_comptime_signature_directives_for_analysis(&mut program)
            .expect_err("pre-barrier pre-pass site must be a compile error");
        assert!(
            error.to_string().contains("no semantic freeze handle"),
            "row-3 named diagnostic missing: {error}"
        );
    }

    /// Dec 52 ordering proof (rejection-matrix row 4): a freeze-boundary
    /// rejection fires at the barrier, BEFORE any annotation handler body
    /// executes. The handler here would leave two observable side effects
    /// (a comptime warning and a hard `error()`); the compile error must be
    /// the freeze rejection and neither side effect may be observed.
    #[test]
    fn freeze_rejection_fires_before_annotation_handler_body_executes() {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::{TypeVar, tyvar_to_annotation};

        // Clear any diagnostics left by other tests on this thread.
        let _ = crate::compiler::comptime_builtins::take_comptime_diagnostics();

        // Poison the unit with partial semantic state: a struct field whose
        // annotation still carries an unresolved inference variable.
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "Poisoned".to_string(),
            (vec!["min".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Poisoned".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "min".to_string(),
                    tyvar_to_annotation(&TypeVar::new("T3".to_string())),
                )]
                .into_iter()
                .collect::<std::collections::HashMap<String, TypeAnnotation>>(),
            },
        );

        let program = parse(
            r#"
annotation marker() on type {
  comptime post(target, ctx) {
    warning("SIDE_EFFECT")
    error("HANDLER_RAN")
  }
}

@marker()
type Probe { id: int }
"#,
        );

        let error = compiler
            .compile(&program)
            .expect_err("partial semantic state must reject compilation at the barrier");
        let message = error.to_string();
        assert!(
            message.contains("unresolved inference variable"),
            "the compile error must be the named freeze rejection, got: {message}"
        );
        assert!(
            !message.contains("HANDLER_RAN"),
            "Dec 52 violated: the handler body executed before the freeze \
             rejection fired: {message}"
        );
        let diagnostics = crate::compiler::comptime_builtins::take_comptime_diagnostics();
        assert!(
            diagnostics
                .iter()
                .all(|d| !d.message.contains("SIDE_EFFECT")),
            "Dec 52 violated: handler side effect observed: {diagnostics:?}"
        );
    }

    fn single_function(source: &str) -> shape_ast::ast::FunctionDef {
        parse(source)
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(f, _) => Some(f),
                _ => None,
            })
            .expect("single function")
    }

    /// Rejection-matrix row 3 (ADR-009 E3 S3, U11): building the typed
    /// `ctx.original` capability without an installed semantic-freeze handle is
    /// the named compile error. The capability's `FrozenCallable` (B6) is
    /// minted through THE single freeze handle — never a fabricated / partial /
    /// string descriptor.
    #[test]
    fn ctx_original_capability_without_freeze_handle_is_the_named_row3_compile_error() {
        let func_def = single_function("fn add_ten(x: int) -> int { return x + 10 }");
        let compiler = BytecodeCompiler::new();
        let error = compiler
            .build_original_capability(&func_def, "\u{1}shadow".to_string())
            .expect_err("pre-barrier capability build must be a compile error");
        assert!(
            error.to_string().contains("no semantic freeze handle"),
            "row-3 named diagnostic missing: {error}"
        );
    }

    /// Positive twin: with the freeze installed, the capability carries a real
    /// typed `FrozenCallable` identity (not `INVALID`) — proving `ctx.original`
    /// is a typed callable minted through the freeze, not a string/Any surface.
    #[test]
    fn ctx_original_capability_with_freeze_is_a_typed_frozen_callable() {
        use crate::compiler::comptime_builtins::FrozenTypeIdentity;
        let func_def = single_function("fn add_ten(x: int) -> int { return x + 10 }");
        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(&func_def)
            .expect("signature registers");
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        let capability = compiler
            .build_original_capability(&func_def, "\u{1}shadow".to_string())
            .expect("post-barrier capability builds");
        assert_eq!(capability.shadow_name(), "\u{1}shadow");
        assert_ne!(
            capability.callable(),
            FrozenTypeIdentity::INVALID,
            "ctx.original must be a real typed FrozenCallable identity"
        );
    }
}

// ADR-009 ticket D1 (slice S2) — provenance stamping on the existing
// two-phase extend path + identity-keyed dedup.
//
// Decision 68 / Decision 67 invariant 5: every generated declaration is
// reserved in the compiler's `GeneratedSymbolTable` under a content-derived
// `SymbolId` with full `ExpansionIdentity` + `GeneratedOrigin`. The
// speculative pre-pass (`materialize_computed_comptime_extends`) and the
// authoritative pass-2 compile (`apply_comptime_extend` /
// `apply_comptime_extend_items`) are the SAME application identity — one
// record, never two, never a doubled diagnostic. Dedup is keyed on the
// expansion identity; name lookups are a derived view into the table.
#[cfg(test)]
mod s2_expansion_provenance_tests {
    use super::BytecodeCompiler;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, ExpansionSite,
        GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC, GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC,
        GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC, GeneratorRef, TargetIdentity,
    };
    use shape_ast::ast::Span;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    fn first_extend(program: &shape_ast::ast::Program) -> shape_ast::ast::ExtendStatement {
        program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item")
    }

    /// A hand-built expansion site for driving the pass-2 registration entry
    /// point directly (the real enforcement point for rows 1 and 3).
    fn test_site(application_span: Span) -> ExpansionSite {
        let no_args: [(&str, &str); 0] = [];
        ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:test_gen:comptime-post"),
                ApplicationId::from_canonical_descriptor("application:test:10:20"),
                TargetIdentity::from_canonical_descriptor("type:UserRow"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&no_args),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            application_span,
            // Generator-definition span: a distinct real span so S4 tests
            // can tell the generator anchor apart from the application.
            Span::new(2, 8),
        )
    }

    /// Risk-7 agreement proof, extend-method shape: the generated method is
    /// registered by the speculative pre-pass and re-seen by the
    /// authoritative pass-2 compile under the SAME `ExpansionIdentity` —
    /// exactly ONE record in the generated-symbol table (a disagreement
    /// would either double the table or trip the row-2 conflict error and
    /// fail compilation).
    #[test]
    fn prepass_and_pass2_agree_on_one_expansion_identity_for_generated_extend_method() {
        let program = parse(
            r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend (extend_method_literal(target.name, "answer", "int", 42))
  }
}

@gen()
type Point { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("generated extend method compiles through both phases");
        assert_eq!(
            compiler.generated_symbols.len(),
            1,
            "pre-pass and pass-2 must agree on ONE identity for one generated decl"
        );
        let id = compiler
            .generated_symbols
            .symbol_for_name("Point.answer")
            .expect("generated method resolves through the derived name view");
        let origin = compiler
            .generated_symbols
            .origin_of(id)
            .expect("reserved identity has full provenance");
        assert!(
            origin
                .expansion
                .target
                .canonical_descriptor()
                .contains("Point"),
            "target identity must name the annotated type, got {:?}",
            origin.expansion.target
        );
        assert!(
            origin
                .expansion
                .generator
                .canonical_descriptor()
                .contains("gen"),
            "generator identity must name the annotation, got {:?}",
            origin.expansion.generator
        );
        assert_ne!(
            origin.source_anchor.span(),
            Span::DUMMY,
            "generated decls anchor at the real application span, never DUMMY"
        );
    }

    /// Risk-7 agreement proof, free-function shape (the §4.5.1 pre-pass
    /// visibility case): `fn main` resolves the generated free function AND
    /// the table holds exactly one record for it after pass-2 re-runs the
    /// same handler.
    #[test]
    fn prepass_and_pass2_agree_on_one_expansion_identity_for_generated_free_function() {
        let program = parse(
            r#"
annotation gen2() on type {
  comptime post(target, ctx) {
    extend (item_fn("generated_flag", "int", 7))
  }
}

@gen2()
type Point { id: int }

fn main() -> int { generated_flag() }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("generated free function compiles through both phases");
        assert_eq!(
            compiler.generated_symbols.len(),
            1,
            "pre-pass and pass-2 must agree on ONE identity for one generated decl"
        );
        compiler
            .generated_symbols
            .symbol_for_name("generated_flag")
            .expect("generated free function resolves through the derived name view");
    }

    /// Rejection-matrix row 2: a second, conflicting definition for one
    /// generated symbol name — two DIFFERENT applications each generating
    /// `fn clash()` — is the named compile error carrying expansion
    /// provenance, not a silent first-wins dedup.
    #[test]
    fn conflicting_generated_name_across_applications_is_the_named_row2_compile_error() {
        let program = parse(
            r#"
annotation dup() on type {
  comptime post(target, ctx) {
    extend (item_fn("clash", "int", 1))
  }
}

@dup()
type A { id: int }

@dup()
type B { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .compile_in_place(&program)
            .expect_err("two applications generating one symbol name must conflict");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC),
            "row-2 named diagnostic missing: {message}"
        );
        assert!(
            message.contains("clash"),
            "conflict diagnostic must name the generated symbol: {message}"
        );
        assert!(
            message.contains("annotation:dup"),
            "conflict diagnostic must carry generator provenance: {message}"
        );
    }

    /// Rejection-matrix row 3: the SAME full application identity expanded
    /// twice with CONFLICTING output (same generated method name, different
    /// body) is the named duplicate-identity compile error at the real
    /// registration entry point.
    #[test]
    fn same_application_identity_with_conflicting_output_is_the_named_row3_compile_error() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&parse("type UserRow { id: int }"))
            .expect("target type compiles");

        let first = first_extend(&parse("extend UserRow { method row() -> int { 1 } }"));
        let second = first_extend(&parse("extend UserRow { method row() -> int { 2 } }"));
        let site = test_site(Span::new(10, 20));

        compiler
            .apply_comptime_extend(first, "UserRow", &site)
            .expect("first expansion of the identity reserves and compiles");
        let error = compiler
            .apply_comptime_extend(second, "UserRow", &site)
            .expect_err("conflicting output for one reserved identity must be refused");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC),
            "row-3 named diagnostic missing: {message}"
        );
    }

    /// Rejection-matrix row 1 (Dec 68 required rejection): a generated
    /// declaration whose application anchor is `Span::DUMMY` — the named
    /// `UserRow` + dummy-span node — is refused at the registration entry
    /// point with the named diagnostic, BEFORE any registration or compile.
    #[test]
    fn generated_decl_anchored_at_dummy_span_is_the_named_row1_compile_error() {
        let mut compiler = BytecodeCompiler::new();
        let extend = first_extend(&parse("extend UserRow { method row() -> int { 1 } }"));
        let site = test_site(Span::DUMMY);

        let error = compiler
            .apply_comptime_extend(extend, "UserRow", &site)
            .expect_err("a dummy-anchored generated decl must be refused");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC),
            "row-1 named diagnostic missing: {message}"
        );
        assert_eq!(
            compiler.generated_symbols.len(),
            0,
            "nothing may be reserved for an unanchorable generated decl"
        );
    }
}

// ADR-009 E3 (slice S4, legacy class U11) — the typed target-OWNER descriptor
// resolution that replaces the deleted magic `TypeName == "target"` literal
// substitution. `extend <target>` is resolved against the handler's POSITION-0
// binding (never a fixed `"target"` spelling) and the typed `TargetOwner`
// (name + `NominalShape`), so a user type literally named `target` resolves
// NOMINALLY when the handler's first parameter is spelled differently.
#[cfg(test)]
mod s4_target_owner_tests {
    use super::BytecodeCompiler;
    use crate::compiler::comptime_builtins::ComptimeDirective;
    use shape_ast::ast::TypeName;
    use shape_runtime::annotation_context::TargetOwner;
    use shape_runtime::comptime_reflection::NominalShape;

    fn extend_directive(source: &str) -> ComptimeDirective {
        let program = shape_ast::parse_program(source).expect("test program parses");
        let extend = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item");
        ComptimeDirective::Extend(extend)
    }

    fn extend_head(directive: &ComptimeDirective) -> &str {
        let ComptimeDirective::Extend(extend) = directive else {
            panic!("expected an Extend directive");
        };
        match &extend.type_name {
            TypeName::Simple(name) => name.as_str(),
            TypeName::Generic { name, .. } => name.as_str(),
        }
    }

    /// Position-bound substitution: when the `extend` head names the handler's
    /// POSITION-0 target binding, it resolves to the typed owner's nominal name.
    #[test]
    fn extend_head_matching_position0_binding_resolves_to_owner() {
        let mut directives = vec![extend_directive(
            "extend target { method m() -> int { 1 } }",
        )];
        let owner = TargetOwner::new("Alpha", NominalShape::Struct);
        BytecodeCompiler::resolve_extend_owner_placeholder(&mut directives, &owner, Some("target"));
        assert_eq!(
            extend_head(&directives[0]),
            "Alpha",
            "an `extend <target>` head matching the position-0 binding resolves to the owner"
        );
    }

    /// Rejection-matrix row 2/3 — NO MAGIC `"target"` SPELLING. When the
    /// handler's position-0 target parameter is spelled DIFFERENTLY (`t`), a
    /// user type literally named `target` is left untouched by the resolver and
    /// resolves NOMINALLY through the ordinary type-name table. The deleted
    /// literal `TypeName == "target"` match would have hijacked it.
    #[test]
    fn literal_target_type_is_nominal_when_binding_differs() {
        let mut directives = vec![extend_directive(
            "extend target { method m() -> int { 1 } }",
        )];
        let owner = TargetOwner::new("Alpha", NominalShape::Struct);
        BytecodeCompiler::resolve_extend_owner_placeholder(&mut directives, &owner, Some("t"));
        assert_eq!(
            extend_head(&directives[0]),
            "target",
            "a real `target` type resolves nominally when the target binding is spelled `t`"
        );
    }

    /// The position-0 binding itself is what resolves — an `extend <t>` head
    /// binds to the owner when the handler names its target parameter `t`.
    #[test]
    fn extend_head_binds_by_position_not_by_the_word_target() {
        let mut directives = vec![extend_directive("extend t { method m() -> int { 1 } }")];
        let owner = TargetOwner::new("Beta", NominalShape::Struct);
        BytecodeCompiler::resolve_extend_owner_placeholder(&mut directives, &owner, Some("t"));
        assert_eq!(
            extend_head(&directives[0]),
            "Beta",
            "the placeholder is bound by POSITION (the handler's first param), not the literal `target`"
        );
    }

    /// A `None` binding (no positional target — e.g. a bare `comptime {}` block)
    /// leaves every `extend` head untouched.
    #[test]
    fn no_binding_leaves_extend_head_untouched() {
        let mut directives = vec![extend_directive(
            "extend target { method m() -> int { 1 } }",
        )];
        let owner = TargetOwner::new("Alpha", NominalShape::Struct);
        BytecodeCompiler::resolve_extend_owner_placeholder(&mut directives, &owner, None);
        assert_eq!(extend_head(&directives[0]), "target");
    }

    /// The owner is a TYPED descriptor (canonical nominal name + declaration
    /// shape, ADR-009 B5), never a `TypeName` string.
    #[test]
    fn target_owner_is_a_typed_nominal_descriptor() {
        let owner = TargetOwner::new("Widget", NominalShape::Struct);
        assert_eq!(owner.name(), "Widget");
        assert_eq!(owner.shape(), NominalShape::Struct);
    }
}

// ADR-009 ticket D1 (slice S3) — real source anchors on generated
// declarations.
//
// Decision 68: generated text and dummy spans are not semantic
// representations. Every generated declaration the compiler registers must
// carry spans that resolve (via `span_to_source_location`) to a REAL
// location in the compiling file — the annotation-application site for
// expansion-emitted decls, the handler definition for annotation-handler
// wrappers. `Span::DUMMY` numerically equals a legitimate offset-0 span, so
// every assertion here is on the RESOLVED line, never a `{0,0}` comparison.
#[cfg(test)]
mod s3_source_anchor_tests {
    use super::BytecodeCompiler;
    use shape_ast::ast::Span;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Compile `source` with source text installed so spans resolve to real
    /// line/column locations. `compile_in_place` moves `source_text` into
    /// the program's debug info at the end of compilation, so the source is
    /// re-installed afterwards for the test's own resolutions.
    fn compiled_with_source(source: &str) -> BytecodeCompiler {
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("test program compiles");
        compiler.set_source(source);
        compiler
    }

    /// 1-indexed line of the first occurrence of `needle` in `source`.
    fn line_of(source: &str, needle: &str) -> usize {
        let offset = source.find(needle).expect("needle present in source");
        source[..offset].chars().filter(|c| *c == '\n').count() + 1
    }

    fn resolved_line(compiler: &BytecodeCompiler, span: Span) -> usize {
        compiler.span_to_source_location(span).line
    }

    /// A generated `extend target { method }` declaration (the DIRECT
    /// handler-AST directive shape) registers with its name span anchored at
    /// the annotation-application site — not `Span::DUMMY` resolving to
    /// line 1 of the wrong text.
    #[test]
    fn generated_extend_target_method_name_span_anchors_at_the_application_site() {
        let source = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

@gen()
type Point { id: int }
"#;
        let compiler = compiled_with_source(source);
        let func_def = compiler
            .function_defs
            .get("Point.answer")
            .expect("generated method is registered");
        let application_line = line_of(source, "@gen()");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "generated method name span must resolve to the @gen() application line"
        );
    }

    /// ADR-009 E2 #18 5b Part B — producer-route METHOD span pin (replaces the
    /// retired snippet-span test whose subject was the deleted `extend (f"…")`
    /// route). A generated method from the TYPED producer (extend_method_literal)
    /// carries Span::default() scaffolding that the shared check sequence re-bases
    /// to the application span; the registered declaration must anchor there. This
    /// is the surviving-route residual required to retire the deleted snippet test.
    #[test]
    fn generated_producer_extend_method_name_span_anchors_at_the_application_site() {
        let source = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend (extend_method_literal(target.name, "answer", "int", 42))
  }
}

@gen()
type Point { id: int }
"#;
        let compiler = compiled_with_source(source);
        let func_def = compiler
            .function_defs
            .get("Point.answer")
            .expect("generated method is registered");
        let application_line = line_of(source, "@gen()");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "snippet-parsed generated method must anchor at the application line"
        );
    }

    /// ADR-009 E2 #18 5b Part B — producer-route FREE-FUNCTION span pin (replaces
    /// the retired snippet-span test whose subject was the deleted
    /// `mod __module_probe__` reparse route). A generated free function from the
    /// TYPED producer (item_fn) anchors at the application site, and its
    /// `GeneratedOrigin` source anchor resolves to the SAME location — the identity
    /// table and the registered declaration agree on one real anchor. The
    /// surviving-route residual required to retire the deleted snippet test.
    #[test]
    fn generated_producer_free_function_anchors_at_the_application_site() {
        let source = r#"
annotation gen2() on type {
  comptime post(target, ctx) {
    extend (item_fn("generated_flag", "int", 7))
  }
}

@gen2()
type Point { id: int }

fn main() -> int { generated_flag() }
"#;
        let compiler = compiled_with_source(source);
        let application_line = line_of(source, "@gen2()");

        let func_def = compiler
            .function_defs
            .get("generated_flag")
            .expect("generated free function is registered");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "generated free-function name span must anchor at the application line, \
             not at snippet-relative offsets"
        );

        let id = compiler
            .generated_symbols
            .symbol_for_name("generated_flag")
            .expect("generated free function has an issued SymbolId");
        let origin = compiler
            .generated_symbols
            .origin_of(id)
            .expect("issued SymbolId has full provenance");
        assert_eq!(
            resolved_line(&compiler, origin.source_anchor.span()),
            application_line,
            "GeneratedOrigin source anchor must resolve to the same application line"
        );
    }

    /// `desugar_extend_method` carries the METHOD's own span onto the
    /// desugared FunctionDef (name span + every synthesized type-param
    /// span) — the hand-written extend path's real anchor.
    #[test]
    fn desugared_extend_method_carries_the_method_span_not_dummy() {
        let source = "extend Vec<T> {\n  method always_one() -> int { 1 }\n}";
        let program = parse(source);
        let extend = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item");
        let method = &extend.methods[0];
        assert_ne!(method.span, Span::DUMMY, "parser anchors the method");

        let compiler = BytecodeCompiler::new();
        let func_def = compiler
            .desugar_extend_method(method, &extend.type_name)
            .expect("method desugars");
        assert_eq!(
            func_def.name_span, method.span,
            "desugared extend method must carry the method's own span"
        );
        let type_params = func_def
            .type_params
            .expect("generic target has type params");
        assert!(!type_params.is_empty(), "extend Vec<T> synthesizes T");
        for tp in &type_params {
            let span = match tp {
                shape_ast::ast::TypeParam::Type { span, .. } => *span,
                shape_ast::ast::TypeParam::Const { span, .. } => *span,
            };
            assert_eq!(
                span,
                method.span,
                "synthesized type param `{}` must anchor at the method span",
                tp.name()
            );
        }
    }

    // ADR-009 C3-S6 completion: `annotation_handler_wrapper_anchors_at_the_
    // handler_definition` DELETED — it anchored the LEGACY specialized
    // before-handler (a zero-param `logged()` + `before(args, ctx)` fixture
    // that no longer compiles post-collapse). The typed weave carries its own
    // span-anchoring pins (template_specialization/weave.rs).
}

// ADR-009 ticket D1 (slice S4) — the shared compiler query surface for
// generated symbols + provenance-carrying diagnostics.
//
// Decision 66 closing rule: tooling resolves generated declarations through
// COMPILER QUERY RESULTS — {SymbolId, checked-decl location, application
// location, generator-definition location} — answered from the S2 identity
// table only, never by text scan and never by a second expansion run.
// Rejection row 7: a diagnostic raised on a generated declaration carries
// generated-node + application-site + generator-definition locations.
#[cfg(test)]
mod s4_query_surface_and_diagnostics_tests {
    use super::BytecodeCompiler;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, ExpansionSite,
        GeneratorRef, TargetIdentity,
    };
    use shape_ast::ast::Span;
    use shape_ast::error::ShapeError;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Compile `source` with source text installed so spans resolve to real
    /// line/column locations (see the S3 test-mod helper: `compile_in_place`
    /// moves `source_text` into debug info, so it is re-installed for the
    /// test's own resolutions).
    fn compiled_with_source(source: &str) -> BytecodeCompiler {
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("test program compiles");
        compiler.set_source(source);
        compiler
    }

    /// 1-indexed line of the first occurrence of `needle` in `source`.
    fn line_of(source: &str, needle: &str) -> usize {
        let offset = source.find(needle).expect("needle present in source");
        source[..offset].chars().filter(|c| *c == '\n').count() + 1
    }

    fn resolved_line(compiler: &BytecodeCompiler, span: Span) -> usize {
        compiler.span_to_source_location(span).line
    }

    const GENERATED_METHOD_FIXTURE: &str = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
      method double() -> int { 84 }
    }
  }
}

@gen()
type Point { id: int }
"#;

    /// The query surface resolves a generated declaration NAME to its full
    /// provenance: SymbolId + checked-decl + application + generator
    /// locations, each resolving to the right REAL source line — answered
    /// from the identity table via `generated_symbol_query()` alone.
    #[test]
    fn query_surface_resolves_generated_method_provenance_to_real_lines() {
        let source = GENERATED_METHOD_FIXTURE;
        let compiler = compiled_with_source(source);
        let application_line = line_of(source, "@gen()");
        let generator_line = line_of(source, "comptime post(target, ctx)");

        let provenance = compiler
            .generated_symbol_query()
            .provenance_for_name("Point.answer")
            .expect("generated method resolves through the query surface");
        assert_eq!(provenance.decl_name, "Point.answer");
        assert_eq!(
            provenance.node_path.render(),
            "extend:Point/method:answer",
            "node path identifies the generated node"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.checked_decl.span()),
            application_line,
            "checked-decl location resolves to the application line (S3 anchoring)"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.application.span()),
            application_line,
            "application location resolves to the @gen() line"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.generator.span()),
            generator_line,
            "generator-definition location resolves to the comptime handler line"
        );

        let by_id = compiler
            .generated_symbol_query()
            .provenance_of(provenance.symbol)
            .expect("issued SymbolId resolves to the same provenance");
        assert_eq!(by_id, provenance, "name view and SymbolId view agree");
    }

    /// The query surface lists every generated symbol (workspace-symbol
    /// consumption) in deterministic order, and resolves a position inside
    /// the checked-decl anchor to the generated declarations anchored there.
    #[test]
    fn query_surface_lists_and_position_resolves_generated_symbols() {
        let source = GENERATED_METHOD_FIXTURE;
        let compiler = compiled_with_source(source);
        let query = compiler.generated_symbol_query();

        let names: Vec<&str> = query
            .generated_symbols()
            .iter()
            .map(|provenance| provenance.decl_name)
            .collect();
        assert_eq!(
            names,
            vec!["Point.answer", "Point.double"],
            "workspace-symbol listing enumerates every generated decl deterministically"
        );

        let anchor = query
            .provenance_for_name("Point.answer")
            .expect("generated method resolves")
            .application;
        let at_application = query.symbols_at(anchor.file_id(), anchor.span().start);
        assert_eq!(
            at_application
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.answer", "Point.double"],
            "a position on the application resolves every decl anchored there"
        );

        let type_offset = source.find("type Point").expect("fixture has the type");
        assert!(
            query.symbols_at(anchor.file_id(), type_offset).is_empty(),
            "a position outside every generated anchor resolves to nothing"
        );
    }

    /// Rejection row 7: an error raised INSIDE a generated method body
    /// (here: the generated body calls an undefined function) surfaces as
    /// the C0003 diagnostic carrying THREE location-bearing notes —
    /// generated node, application site, generator definition — each
    /// resolving to its real line.
    #[test]
    fn generated_body_failure_carries_three_provenance_note_locations() {
        let source = "type UserRow { id: int }\n// application line\n// generator line\n";
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("target type compiles");
        compiler.set_source(source);

        let application_offset = source.find("// application line").expect("marker");
        let generator_offset = source.find("// generator line").expect("marker");
        let application_span = Span::new(application_offset, application_offset + 4);
        let generator_span = Span::new(generator_offset, generator_offset + 4);
        let application_line = line_of(source, "// application line");
        let generator_line = line_of(source, "// generator line");

        let no_args: [(&str, &str); 0] = [];
        let site = ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:broken_gen:comptime-post"),
                ApplicationId::from_canonical_descriptor("application:test:row7"),
                TargetIdentity::from_canonical_descriptor("type:UserRow"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&no_args),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            application_span,
            generator_span,
        );

        let extend_program =
            parse("extend UserRow { method broken() -> int { missing_helper() } }");
        let extend = extend_program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item");

        let err = compiler
            .apply_comptime_extend(extend, "UserRow", &site)
            .expect_err("a generated body calling an undefined function must fail");
        let ShapeError::SemanticError {
            message,
            location: Some(location),
        } = &err
        else {
            panic!("row-7 failure must be a located SemanticError, got {err:?}");
        };
        assert!(
            message.contains("error in generated declaration `UserRow.broken`"),
            "row-7 message must name the generated declaration: {message}"
        );
        assert_eq!(
            location.line, application_line,
            "row-7 primary location anchors at the application site"
        );
        assert_eq!(
            location.notes.len(),
            3,
            "row-7 diagnostic carries exactly the three provenance notes: {:?}",
            location.notes
        );
        let generated_note = &location.notes[0];
        assert!(
            generated_note
                .message
                .contains("generated node extend:UserRow/method:broken"),
            "generated-node note must carry the node path: {}",
            generated_note.message
        );
        assert_eq!(
            generated_note
                .location
                .as_ref()
                .expect("generated-node note has a location")
                .line,
            application_line,
            "generated-node note resolves to the checked-decl (application) line"
        );
        let application_note = &location.notes[1];
        assert!(
            application_note
                .message
                .contains("generated from this application site"),
            "application note missing: {}",
            application_note.message
        );
        assert_eq!(
            application_note
                .location
                .as_ref()
                .expect("application note has a location")
                .line,
            application_line,
        );
        let generator_note = &location.notes[2];
        assert!(
            generator_note.message.contains("generator defined here"),
            "generator note missing: {}",
            generator_note.message
        );
        assert_eq!(
            generator_note
                .location
                .as_ref()
                .expect("generator note has a location")
                .line,
            generator_line,
            "generator note resolves to the generator-definition line"
        );
    }

    /// End-to-end row 7: the SAME provenance-carrying diagnostic surfaces
    /// through the full two-phase pipeline when an annotation handler
    /// generates a method whose body fails to compile. Runs under the
    /// RecoverAll diagnostic modes (the LSP configuration) so the pipeline
    /// reaches pass-2's generated-body compile after the analyzer has
    /// already recorded its own view of the broken body; the row-7
    /// diagnostic must be among the surfaced errors WITH its three
    /// location-bearing notes intact (the outer directive-processing wrap
    /// must not flatten them to a string).
    #[test]
    fn end_to_end_generated_body_failure_carries_provenance() {
        let source = r#"
annotation bad_gen() on type {
  comptime post(target, ctx) {
    extend target {
      method broken() -> int { missing_helper() }
    }
  }
}

@bad_gen()
type Point { id: int }
"#;
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_type_diagnostic_mode(crate::compiler::TypeDiagnosticMode::RecoverAll);
        compiler.set_compile_diagnostic_mode(crate::compiler::CompileDiagnosticMode::RecoverAll);
        compiler.set_source(source);
        let err = compiler
            .compile_in_place(&program)
            .expect_err("generated body calling an undefined function must fail the compile");

        fn flatten<'a>(e: &'a ShapeError, out: &mut Vec<&'a ShapeError>) {
            if let ShapeError::MultiError(errors) = e {
                for inner in errors {
                    flatten(inner, out);
                }
            } else {
                out.push(e);
            }
        }
        let mut flat = Vec::new();
        flatten(&err, &mut flat);
        let provenance_error = flat
            .iter()
            .find_map(|e| match e {
                ShapeError::SemanticError {
                    message,
                    location: Some(location),
                } if message.contains("error in generated declaration `Point.broken`") => {
                    Some(location)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "end-to-end failure must include the provenance-carrying \
                     generated-decl diagnostic, got: {flat:?}"
                )
            });
        assert_eq!(
            provenance_error.notes.len(),
            3,
            "the three provenance notes must survive the full pipeline: {:?}",
            provenance_error.notes
        );
    }
}

// ADR-009 E3 (slice S1) — FUNCTION-target discovery parity. A
// `targets: [function]` annotation whose comptime handler emits
// `extend ExplicitType { … }` must enter the SAME executed declaration-
// discovery fixed point (`materialize_computed_comptime_extends`) as a
// type-target one, so the generated method is recorded in the executed
// authority (`generated_analysis_items`, read by the LSP + analyzer + every
// user body) — never via a parallel non-evaluating AST scan. The type-target
// path already discovered; this closes the function-target gap.
#[cfg(test)]
mod e3_function_target_discovery_tests {
    use crate::compiler::executed_generated_items;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// The generated method names an executed-discovery `extend` block records
    /// for `type_name` (across all generated extend items).
    fn discovered_extend_methods(items: &[shape_ast::ast::Item], type_name: &str) -> Vec<String> {
        let mut methods = Vec::new();
        for item in items {
            if let shape_ast::ast::Item::Extend(extend, _) = item {
                let name = match &extend.type_name {
                    shape_ast::ast::TypeName::Simple(n) => n.to_string(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.to_string(),
                };
                if name == type_name {
                    methods.extend(extend.methods.iter().map(|m| m.name.clone()));
                }
            }
        }
        methods
    }

    /// The parity gap: a `targets: [function]` handler that `extend`s an
    /// explicit (builtin) type records the generated method in the executed
    /// discovery output — exactly the `extend Number { method doubled }` shape
    /// the LSP extraction test exercises, proven here at the compiler tier.
    #[test]
    fn function_target_extend_explicit_type_enters_discovery() {
        let program = parse(
            r#"
annotation add_number_method() on function {
    comptime post(target, ctx) {
        extend Number {
            method doubled() { self * 2.0 }
        }
    }
}
@add_number_method()
fn marker() { 0 }
"#,
        );
        let methods = discovered_extend_methods(&executed_generated_items(&program), "Number");
        assert!(
            methods.iter().any(|m| m == "doubled"),
            "function-target `extend Number` must be recorded in executed discovery; got {methods:?}"
        );
    }

    /// A function-target handler that `extend`s a USER type is discovered the
    /// same way (the explicit-type case is not builtin-specific).
    #[test]
    fn function_target_extend_user_type_enters_discovery() {
        let program = parse(
            r#"
type Widget { id: int }
annotation add_label() on function {
    comptime post(target, ctx) {
        extend Widget {
            method label() -> string { f"widget-{self.id}" }
        }
    }
}
@add_label()
fn register() -> int { 0 }
"#,
        );
        let methods = discovered_extend_methods(&executed_generated_items(&program), "Widget");
        assert!(
            methods.iter().any(|m| m == "label"),
            "function-target `extend Widget` must be recorded in executed discovery; got {methods:?}"
        );
    }

    /// The annotation DEFINITION alone (never applied to a function) generates
    /// nothing — the discovery pass runs only applied handlers (the run-once
    /// memo claims one application per site), so no speculative pollution.
    #[test]
    fn unapplied_function_target_annotation_generates_nothing() {
        let program = parse(
            r#"
annotation add_number_method() on function {
    comptime post(target, ctx) {
        extend Number {
            method doubled() { self * 2.0 }
        }
    }
}
fn marker() { 0 }
"#,
        );
        let methods = discovered_extend_methods(&executed_generated_items(&program), "Number");
        assert!(
            methods.is_empty(),
            "an unapplied function-target annotation must not generate discovery items; got {methods:?}"
        );
    }
}
