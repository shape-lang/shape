//! ADR-009 C3 #14 (slice 2, S2c) — the typed-AST weave: materialize the
//! staged hook installs for one target as a GENERATED ORDINARY TYPED AST
//! wrapper + a journaled hygienic impl shadow.
//!
//! # The C3-G6 SMALL shape (slice-0 §2 — binding)
//!
//! The wrapper is an ordinary typed AST `FunctionDef` compiled through the
//! ORDINARY pipeline, so bytecode AND MIR derive from the same wrapped
//! definition — full native, no `mir_data` suppression. Un-suppressing
//! `mir_data` on the deleted LEGACY raw-bytecode weave was
//! MEASURED-FORBIDDEN (slice-0 §2.3: silent VM≠JIT divergence, hooks
//! silently skipped); the S6 capstone deleted that weave, its selector and
//! its mir_data suppression whole (deletion-fate: the C3-G7 charter), so
//! this typed weave is THE ONLY weave and every woven wrapper compiles as
//! an ordinary fn with the ordinary mir-attach tail in
//! `compile_function_inner`.
//!
//! # The weave shape
//!
//! Materialized ONCE per target, after the LAST handler + body directives
//! (so it wraps the FINAL — possibly `replace body`-edited — definition):
//!
//! 1. The final target body moves under an unspellable hygienic shadow name
//!    ([`HygienicRole::TemplateWeaveImplBody`], the C3 successor of the
//!    deleted legacy hook-impl shadow role), reserved through
//!    `reserve_generated_decl_journaled` (the `CheckedReplaceBody`
//!    shadow-construction precedent) so a failing later compile rolls the
//!    reservation back with the rest of the open C2 `InstallTransaction` —
//!    and compiled through the ordinary NESTED `compile_function` (the
//!    monomorphization-ride pattern), which gives the shadow its own
//!    mir-attach (the slice-0 "(ii) mir_data for hygienic impl emissions"
//!    plumbing: on this path the shadow IS an ordinary fn, so the ordinary
//!    tail attaches its MIR — no authority split, no source-keyed remap).
//! 2. The wrapper body replaces `func_def`'s body IN PLACE (same name, same
//!    signature), and `compile_function_inner` continues: `before` chain in
//!    APPLICATION order (`MutationCarrier::Single`: rebind the one typed
//!    arg through each handler; `MutationCarrier::Aggregate`: bind the
//!    handler's inline-schema aggregate to a local ANNOTATED with the
//!    carrier's field types — the CURRENT target's AST-side spellings,
//!    consumed via `ConcreteType` equality semantics, never spelling
//!    equality — then read `m.a0..m.aN-1` as the next call's args), the
//!    direct impl call to the shadow (awaited for async targets), then the
//!    `after` chain threading the typed result in REVERSE application order
//!    (fix-round-1: the WRAPPING/onion semantic — the first-applied
//!    annotation is the outermost wrapper, so its `after` runs last; this is
//!    the surviving declarative surface's stacked-after semantic, green-
//!    pinned at `annotations_runtime/wrapping.rs`
//!    `stacked_after_hooks_transform_result_in_order`, and no C3 ruling
//!    ordered a change — a dated user ruling can flip this single iteration
//!    order). S3b: capture values are BAKED into the specialized handlers
//!    (`const_lift::bake_captures_into_def`) — the weave passes ONLY the
//!    current Sig args at every handler call site; handler arity == Sig
//!    arity everywhere. OBSERVER installs (fix-round-1 C3-G2 growth:
//!    concrete zero-signature-param void bodies) are called at their chain
//!    position with ZERO arguments — the args/result flow is untouched,
//!    which is what gives zero-param targets a `before` spelling and void
//!    targets an `after` spelling.
//!
//! Wrapper-internal locals are minted under the reserved `__c3_` prefix
//! (`__c3_w{n}`), skipping any (pathological) user parameter spelled with a
//! colliding name.
//!
//! # What this module rejects (named, surface-and-stop)
//!
//! - A target parameter with no simple name (destructuring parameter): the
//!   wrapper forwards parameters BY NAME to the shadow and the handlers; a
//!   destructured parameter has no forwardable spelling. Positive twin:
//!   bind the parameter to a name.
//! - Internal invariants (a non-observer Before install without a carrier, a
//!   result-threading After install on a void target, a generic def reaching
//!   the weave) are internal-error-shaped — the apply seam's rejections make
//!   them unreachable from user code.

use std::collections::HashSet;

use shape_ast::ast::expressions::Expr;
use shape_ast::ast::functions::FunctionDef;
use shape_ast::ast::program::{OwnershipModifier, VarKind, VariableDecl};
use shape_ast::ast::span::Span;
use shape_ast::ast::statements::Statement;
use shape_ast::ast::types::{ObjectTypeField, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_ast::ast::patterns::DestructurePattern;

use super::MutationCarrier;
use super::install_registry::StagedHookInstall;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::{
    GeneratedNodePath, GeneratedOrigin, HygienicRole, SymbolReservation,
};
use crate::compiler::comptime_fragments::checked_template::TemplateHookKind;
use crate::compiler::functions_annotations::generated_free_fn_content;

impl BytecodeCompiler {
    /// The unspellable hygienic registry name of the hook-template weave's
    /// impl shadow for `func_name` (the target's FINAL body). The nonce is a
    /// stable digest of the function name, so re-registration is idempotent
    /// (one weave shadow per woven function; `register_function` dedups by
    /// name) — the `original_body_shadow_name` precedent.
    ///
    /// `pub(in crate::compiler)` (C3-S5c): tests locate the impl shadow BY
    /// ROLE through this ONE producer — never by spelling an SOH string —
    /// so the hygienic rename class that killed the pre-rewrite
    /// `compute___impl` lookup (761469cd) cannot recur.
    pub(in crate::compiler) fn template_weave_impl_name(&self, func_name: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        func_name.hash(&mut hasher);
        self.mint_hygienic_fn_name_stable(HygienicRole::TemplateWeaveImplBody, hasher.finish())
    }

    /// Materialize the staged hook installs for one target (module docs for
    /// the full contract). Mutates `func_def` in place: on return its body
    /// IS the generated wrapper, and the shadow (holding the previous final
    /// body) is registered + compiled under its hygienic identity.
    pub(in crate::compiler) fn materialize_hook_template_weave(
        &mut self,
        func_def: &mut FunctionDef,
        staged: &[StagedHookInstall],
    ) -> Result<()> {
        let internal = |message: String| ShapeError::RuntimeError {
            message,
            location: None,
        };
        let first = staged
            .first()
            .expect("caller gates on a non-empty staged-install accumulator");
        let application_span = first.application_span;

        // Defensive invariant (module docs): unreachable from user code —
        // the apply seam's G8 rejection fires first.
        if func_def
            .type_params
            .as_ref()
            .is_some_and(|params| !params.is_empty())
        {
            return Err(internal(format!(
                "internal error: hook-template weave reached the generic target `{}`; the \
                 C3-G8 rejection fires at the apply seam before any install stages",
                func_def.name
            )));
        }
        // The wrapper forwards parameters by name — a destructuring
        // parameter has no forwardable spelling (module docs).
        let mut param_names: Vec<String> = Vec::with_capacity(func_def.params.len());
        for (index, param) in func_def.params.iter().enumerate() {
            let Some(name) = param.simple_name() else {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot install hook templates on `{}`: parameter {} is a \
                         destructuring pattern, and the generated hook wrapper forwards \
                         parameters by name; bind the parameter to a plain name (destructure \
                         inside the body instead)",
                        func_def.name,
                        index + 1
                    ),
                    location: Some(self.span_to_source_location(application_span)),
                });
            };
            param_names.push(name.to_string());
        }

        // ── 1. The hygienic impl shadow: the target's FINAL body. ──────────
        let shadow_name = self.template_weave_impl_name(&func_def.name);
        let shadow = FunctionDef {
            name: shadow_name.clone(),
            name_span: func_def.name_span,
            declaring_module_path: func_def.declaring_module_path.clone(),
            doc_comment: None,
            params: func_def.params.clone(),
            return_type: func_def.return_type.clone(),
            body: func_def.body.clone(),
            type_params: func_def.type_params.clone(),
            annotations: Vec::new(),
            where_clause: func_def.where_clause.clone(),
            is_async: func_def.is_async,
            is_comptime: func_def.is_comptime,
        };

        // Journaled reservation (the CheckedReplaceBody precedent): origin +
        // anchors derive from the FIRST staged install's ExpansionSite; a
        // failing later compile rolls the reservation back with the open
        // transaction.
        let site = &first.site;
        let content = generated_free_fn_content(&shadow);
        let source_anchor = site.source_anchor().map_err(|message| ShapeError::SemanticError {
            message,
            location: Some(self.span_to_source_location(application_span)),
        })?;
        let generator_anchor = site.generator_anchor().map_err(|message| {
            ShapeError::SemanticError {
                message,
                location: Some(self.span_to_source_location(application_span)),
            }
        })?;
        let origin = GeneratedOrigin {
            expansion: site.identity().clone(),
            node_path: GeneratedNodePath::decl_root(format!("weave_impl:{}", func_def.name)),
            source_anchor,
        };
        match self.reserve_generated_decl_journaled(&shadow_name, origin, content, generator_anchor)
        {
            Ok(SymbolReservation::Fresh(_)) | Ok(SymbolReservation::Reissued(_)) => {}
            Err(message) => {
                return Err(ShapeError::SemanticError {
                    message,
                    location: Some(self.span_to_source_location(application_span)),
                });
            }
        }

        // The shadow's body is USER source carried verbatim: its interior
        // closures keep ordinary capture inference (unstamped — the
        // gate-totality G4 negative-control contract). Record the shadow
        // name so the capture-surface debug crosscheck excludes this class
        // (generated-by-reservation, user-source-by-content) from its
        // "generated decls carry stamped closures" name-view heuristic.
        self.template_weave_shadow_names.insert(shadow_name.clone());

        // Register + compile the shadow through the ordinary NESTED
        // `compile_function` (the monomorphization-ride pattern, cache.rs:
        // save/restore the per-function ephemeral state around the nested
        // compile). The ordinary pipeline gives the shadow bytecode AND its
        // own mir-attach tail — the slice-0 "(ii)" plumbing.
        self.register_function(&shadow)?;
        let saved_closure_function_ids = std::mem::take(&mut self.closure_function_ids);
        let saved_local_concrete_facts =
            std::mem::take(&mut self.current_function_local_concrete_facts);
        let saved_local_binding_spans = std::mem::take(&mut self.local_binding_spans);
        let shadow_result = self.compile_function(&shadow);
        self.closure_function_ids = saved_closure_function_ids;
        self.current_function_local_concrete_facts = saved_local_concrete_facts;
        self.local_binding_spans = saved_local_binding_spans;
        shadow_result?;

        // ── 2. The generated wrapper body, swapped into `func_def`. ────────
        let span = Span::default();
        let taken: HashSet<String> = param_names.iter().cloned().collect();
        let mut counter = 0usize;
        let mut fresh_local = move |taken: &HashSet<String>| loop {
            let candidate = format!("__c3_w{counter}");
            counter += 1;
            if !taken.contains(&candidate) {
                return candidate;
            }
        };
        let ident = |name: &str| Expr::Identifier(name.to_string(), span);
        let call = |name: &str, args: Vec<Expr>| Expr::FunctionCall {
            name: name.to_string(),
            const_args: Vec::new(),
            args,
            named_args: Vec::new(),
            span,
        };
        let decl = |name: &str, annotation: Option<TypeAnnotation>, value: Expr| {
            Statement::VariableDecl(
                VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier(name.to_string(), span),
                    type_annotation: annotation,
                    value: Some(value),
                    ownership: OwnershipModifier::Inferred,
                },
                span,
            )
        };
        let handler_symbol = |compiler: &Self, install: &StagedHookInstall| -> Result<String> {
            compiler
                .program
                .functions
                .get(install.handler.function_index() as usize)
                .map(|function| function.name.clone())
                .ok_or_else(|| {
                    internal(format!(
                        "internal error: staged hook install (via @{}) resolves to no \
                         registered function at index {}",
                        install.annotation_name,
                        install.handler.function_index()
                    ))
                })
        };

        let mut stmts: Vec<Statement> = Vec::new();
        let mut current_args: Vec<Expr> =
            param_names.iter().map(|name| ident(name)).collect();

        // The `before` chain, in application order.
        for install in staged
            .iter()
            .filter(|install| install.hook_kind == TemplateHookKind::Before)
        {
            let symbol = handler_symbol(self, install)?;
            // OBSERVER install: called with ZERO arguments (S3b — its
            // capture values are BAKED into the handler); the target's
            // arguments thread through UNTOUCHED (module docs).
            if install.handler.is_observer() {
                stmts.push(Statement::Expression(call(&symbol, Vec::new()), span));
                continue;
            }
            let args = current_args.clone();
            match install.handler.carrier() {
                Some(MutationCarrier::Single { annotation }) => {
                    let local = fresh_local(&taken);
                    stmts.push(decl(&local, Some(annotation.clone()), call(&symbol, args)));
                    current_args = vec![ident(&local)];
                }
                Some(MutationCarrier::Aggregate { fields }) => {
                    let local = fresh_local(&taken);
                    let aggregate_annotation = TypeAnnotation::Object(
                        fields
                            .iter()
                            .map(|(name, annotation)| ObjectTypeField {
                                name: name.clone(),
                                optional: false,
                                type_annotation: annotation.clone(),
                                annotations: Vec::new(),
                            })
                            .collect(),
                    );
                    stmts.push(decl(&local, Some(aggregate_annotation), call(&symbol, args)));
                    current_args = fields
                        .iter()
                        .map(|(name, _)| Expr::PropertyAccess {
                            object: Box::new(ident(&local)),
                            property: name.clone(),
                            optional: false,
                            span,
                        })
                        .collect();
                }
                None => {
                    return Err(internal(format!(
                        "internal error: staged `before` install (via @{}) carries no \
                         mutation carrier; specialize_template always attaches one to a \
                         non-observer before handler",
                        install.annotation_name
                    )));
                }
            }
        }

        // The direct impl call (awaited on async targets).
        let impl_call = call(&shadow_name, current_args);
        let impl_expr = if func_def.is_async {
            Expr::Await(Box::new(impl_call), span)
        } else {
            impl_call
        };

        let has_result = !matches!(
            func_def.return_type.as_ref(),
            None | Some(TypeAnnotation::Void)
        );
        let afters: Vec<&StagedHookInstall> = staged
            .iter()
            .filter(|install| install.hook_kind == TemplateHookKind::After)
            .collect();
        if has_result {
            let return_annotation = func_def.return_type.clone();
            let result_local = fresh_local(&taken);
            stmts.push(decl(&result_local, return_annotation.clone(), impl_expr));
            let mut result_expr = ident(&result_local);
            // The `after` chain, in REVERSE application order (the
            // wrapping/onion semantic — module docs), threading the typed
            // result; observers run at their chain position without touching
            // it.
            for install in afters.iter().rev() {
                let symbol = handler_symbol(self, install)?;
                if install.handler.is_observer() {
                    stmts.push(Statement::Expression(call(&symbol, Vec::new()), span));
                    continue;
                }
                let args = vec![result_expr];
                let local = fresh_local(&taken);
                stmts.push(decl(&local, return_annotation.clone(), call(&symbol, args)));
                result_expr = ident(&local);
            }
            stmts.push(Statement::Return(Some(result_expr), span));
        } else {
            // A void target hosts OBSERVER afters only (fix-round-1 C3-G2
            // growth): specialize_template rejects every result-threading
            // after on a void target, so a non-observer here is an internal
            // invariant break.
            for install in &afters {
                if !install.handler.is_observer() {
                    return Err(internal(format!(
                        "internal error: staged result-threading `after` install (via @{}) on \
                         the void target `{}`; specialize_template rejects after-templates \
                         without a typed result unless they are observers",
                        install.annotation_name, func_def.name
                    )));
                }
            }
            stmts.push(Statement::Expression(impl_expr, span));
            for install in afters.iter().rev() {
                let symbol = handler_symbol(self, install)?;
                stmts.push(Statement::Expression(call(&symbol, Vec::new()), span));
            }
        }

        func_def.body = stmts;
        Ok(())
    }
}

// ADR-009 C3 #14 (slice 2, S2c): end-to-end BEHAVIOR pins through the PUBLIC
// comptime API over the full compile path (parse → pre-pass → analyzer →
// pass-2 handler → install → weave) with EXECUTED programs — mutation
// observed in program output, per the S0 §2 "compile-proof alone is banned"
// rule. Plus the mir-attach pins (wrapper AND shadow carry mir_data — the
// C3-G6 SMALL shape observable), the aggregate-kind guard pins, the
// cache-share capture pin, stacking order, replace-body composition, the
// alias-spelling regression, and the rollback probe.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};

    /// Whole-program fixture (the S2b `install_registry` harness shape).
    fn hook_source(body_fns: &str, handler_stmts: &str, targets_and_calls: &str) -> String {
        format!(
            r#"
{body_fns}

annotation hookann() {{
  targets: [function]
  comptime post(target, ctx) {{
    {handler_stmts}
  }}
}}

{targets_and_calls}
"#
        )
    }

    fn compile_source(
        src: &str,
    ) -> (shape_ast::error::Result<()>, crate::compiler::BytecodeCompiler) {
        let program = shape_ast::parse_program(src).expect("fixture parses");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        // S5a: real span→line mapping for the application-site anchoring
        // pins (without source_text every SourceLocation degrades to 1:1).
        compiler.source_text = Some(src.to_string());
        let result = compiler.compile_in_place(&program);
        (result, compiler)
    }

    fn compiled_ok(src: &str) -> crate::compiler::BytecodeCompiler {
        let (result, compiler) = compile_source(src);
        result.expect("fixture must compile");
        compiler
    }

    /// Execute the compiled program's top level and return the final value.
    fn execute_top_level(compiler: &crate::compiler::BytecodeCompiler) -> shape_value::KindedSlot {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        vm.execute(None).expect("program executes")
    }

    fn top_level_i64(src: &str) -> (i64, crate::compiler::BytecodeCompiler) {
        let compiler = compiled_ok(src);
        let value = execute_top_level(&compiler)
            .as_i64()
            .expect("top-level result is an int");
        (value, compiler)
    }

    /// The woven target's registered Function entry.
    fn function_entry<'c>(
        compiler: &'c crate::compiler::BytecodeCompiler,
        name: &str,
    ) -> &'c crate::bytecode::Function {
        let index = compiler.find_function(name).expect("function registered");
        &compiler.program.functions[index]
    }

    /// Every registered weave shadow (unspellable SOH-prefixed name minted
    /// under the `TemplateWeaveImplBody` role for this compile).
    fn shadow_entries<'c>(
        compiler: &'c crate::compiler::BytecodeCompiler,
        target: &str,
    ) -> Vec<&'c crate::bytecode::Function> {
        let shadow_name = compiler.template_weave_impl_name(target);
        compiler
            .program
            .functions
            .iter()
            .filter(|function| function.name == shadow_name)
            .collect()
    }

    // ── end-to-end behavior: before / after / both (concrete templates) ────

    // BEFORE mutation observed in program output: victim(4) → add_one → 5 →
    // impl(5) = 50. A skipped hook yields 40 (value-distinguishing).
    #[test]
    fn concrete_before_mutation_is_observed_in_output() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn add_one(x: int) -> int { return x + 1 }",
            "install(before_hook(add_one, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 50, "the before mutation must be observed (skip ⇒ 40)");

        // The C3-G6 SMALL observables: the woven wrapper compiles as an
        // ORDINARY fn — mir_data attached (the deleted legacy weave's
        // suppression would have left it None) — and the hygienic shadow is
        // registered with its OWN mir_data (the slice-0 "(ii)" plumbing).
        let wrapper = function_entry(&compiler, "victim");
        assert!(
            wrapper.mir_data.is_some(),
            "the woven wrapper must carry mir_data (bytecode AND MIR from the wrapped def)"
        );
        let shadows = shadow_entries(&compiler, "victim");
        assert_eq!(shadows.len(), 1, "exactly one weave shadow per woven target");
        assert!(
            shadows[0].name.starts_with('\u{1}'),
            "the shadow name is unspellable (SOH-prefixed): {:?}",
            shadows[0].name
        );
        assert!(
            shadows[0].mir_data.is_some(),
            "the weave shadow compiles through the ordinary pipeline with its own mir-attach"
        );
        assert_eq!(compiler.hook_install_registry.len(), 1);
    }

    // AFTER mutation observed: victim(4) = 40 → double → 80.
    #[test]
    fn concrete_after_mutation_is_observed_in_output() {
        let (value, _) = top_level_i64(&hook_source(
            "fn double(r: int) -> int { return r * 2 }",
            "install(after_hook(double, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 80, "the after mutation must be observed (skip ⇒ 40)");
    }

    // BEFORE + AFTER on one target: (4+1)*10 = 50 → *2 = 100. Any skipped
    // hook yields a distinguishable value (40 / 50 / 80).
    #[test]
    fn before_and_after_together_compose_around_the_impl() {
        let (value, _) = top_level_i64(&hook_source(
            "fn add_one(x: int) -> int { return x + 1 }\n\
             fn double(r: int) -> int { return r * 2 }",
            "install(before_hook(add_one, []))\n    install(after_hook(double, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 100, "before and after must both fire around the impl");
    }

    // ── the heterogeneous AGGREGATE carrier (polymorphic before) ───────────

    // Arity-2 (int, number) target: the polymorphic before mutates slot 0
    // via the pseudo-tuple (a0 = 4*3 + args.length = 14) while slot 1 flows
    // through TYPED (b = 2.5 keeps the `b > 2.0` branch live) — the wrapper
    // reads the compiler-internal aggregate's fields as the impl call's
    // typed args. Skip ⇒ 104; kind-corrupt b ⇒ 14 or a crash.
    #[test]
    fn aggregate_carrier_weave_delivers_both_typed_slots() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   args[0] = args[0] * 3 + args.length\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int {\n\
             \x20   if b > 2.0 { return a + 100 }\n\
             \x20   return a\n\
             }\n\nvictim(4, 2.5)",
        ));
        assert_eq!(
            value, 114,
            "slot 0 mutated (4*3+2=14) AND slot 1 typed-preserved (2.5 > 2.0 ⇒ +100)"
        );
        let wrapper = function_entry(&compiler, "victim");
        assert!(wrapper.mir_data.is_some(), "the aggregate wrapper stays mir-attached");
    }

    // Arity-1 Single carrier (polymorphic): the mutated arg flows back as
    // the bare typed value — no aggregate exists.
    #[test]
    fn single_carrier_weave_rebinds_the_one_typed_arg() {
        let (value, _) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   args[0] = args[0] * 3\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int) -> int { return a + 1 }\n\nvictim(5)",
        ));
        assert_eq!(value, 16, "5*3 = 15 → impl(15) = 16 (skip ⇒ 6)");
    }

    // ── captures: rule-6 baked specializations (S3b PIN FLIP) ──────────────

    // S3b PIN FLIP (Dec-95 rule 6, ordered by the slice-3 charter; the S2
    // twin pinned ONE shared value-generic handler with call-site literal
    // delivery — both superseded): two annotations install the SAME
    // polymorphic template at the SAME Sig with DIFFERENT capture values ⇒
    // TWO distinct baked specializations (the values are in the `::cfg#`
    // identity segment and BAKED into each handler's prologue — no
    // call-site delivery exists). The executed VALUES prove the bake, not a
    // delivery mechanism: each target observes exactly its own config.
    #[test]
    fn rule6_distinct_scalar_config_gets_distinct_baked_specializations() {
        let src = r#"
fn tmpl<Args>(args: Args, factor: int) -> Args {
    args[0] = args[0] * factor
    return args
}

annotation with3() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", 3)]))
  }
}

annotation with5() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", 5)]))
  }
}

@with3()
fn victim_a(a: int) -> int { return a + 1 }

@with5()
fn victim_b(a: int) -> int { return a + 1 }

victim_a(10) * 1000 + victim_b(10)
"#;
        let (value, compiler) = top_level_i64(src);
        // victim_a: 10*3 = 30 → 31; victim_b: 10*5 = 50 → 51 — BYTE-SAME
        // output as the S2 twin (the flip changes identity, not behavior).
        assert_eq!(value, 31051, "each baked handler holds its own config constant");
        assert_eq!(compiler.hook_install_registry.len(), 2);
        let (row_a, row_b) = (
            &compiler.hook_install_registry[0],
            &compiler.hook_install_registry[1],
        );
        assert_ne!(
            row_a.function_index, row_b.function_index,
            "rule 6: structurally different config = DISTINCT specializations"
        );
        assert_ne!(
            row_a.captures, row_b.captures,
            "the installs differ ONLY in their capture literals"
        );
        // The rule-6 identity observable: both symbols carry the `::cfg#`
        // config segment, and the segments differ.
        assert!(
            row_a.specialized_symbol.contains("::cfg#1::i:3"),
            "row a's symbol carries its config segment: {}",
            row_a.specialized_symbol
        );
        assert!(
            row_b.specialized_symbol.contains("::cfg#1::i:5"),
            "row b's symbol carries its config segment: {}",
            row_b.specialized_symbol
        );
    }

    // Rule-6 SHARE, execution-proven (the S0 §2 posture): two applications
    // with structurally EQUAL config on two same-Sig targets resolve to the
    // SAME baked handler index, and BOTH targets execute correctly through
    // it.
    #[test]
    fn rule6_equal_config_shares_one_baked_specialization() {
        let src = r#"
fn tmpl<Args>(args: Args, factor: int) -> Args {
    args[0] = args[0] * factor
    return args
}

annotation first3() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", 3)]))
  }
}

annotation second3() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", 3)]))
  }
}

@first3()
fn victim_a(a: int) -> int { return a + 1 }

@second3()
fn victim_b(a: int) -> int { return a + 2 }

victim_a(10) * 1000 + victim_b(20)
"#;
        let (value, compiler) = top_level_i64(src);
        // victim_a: 10*3 = 30 → 31; victim_b: 20*3 = 60 → 62.
        assert_eq!(value, 31062, "both targets execute through the shared baked handler");
        assert_eq!(compiler.hook_install_registry.len(), 2);
        assert_eq!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "rule 6: structurally EQUAL config SHARES one specialization"
        );
    }

    // ── stacking: application order ────────────────────────────────────────

    // Two before installs stack in APPLICATION order: (1+10)*2 = 22 →
    // impl(22) = 220. The reversed order would yield (1*2+10)*10 = 120.
    #[test]
    fn stacked_installs_weave_in_application_order() {
        let src = r#"
fn add_ten(x: int) -> int { return x + 10 }
fn mul_two(x: int) -> int { return x * 2 }

annotation first_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_ten, []))
  }
}

annotation second_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(mul_two, []))
  }
}

@first_hook()
@second_hook()
fn victim(a: int) -> int { return a * 10 }

victim(1)
"#;
        let (value, _) = top_level_i64(src);
        assert_eq!(
            value, 220,
            "before chain in application order: (1+10)*2 → impl ⇒ 220 (reversed ⇒ 120)"
        );
    }

    // Two AFTER installs stack in REVERSE application order (fix-round-1:
    // the wrapping/onion semantic — the first-applied annotation is the
    // OUTERMOST wrapper, so its after runs LAST; the surviving declarative
    // surface's stacked-after semantic, wrapping.rs
    // stacked_after_hooks_transform_result_in_order): impl(1) = 10 →
    // mul_two 20 → add_ten 30 (application order would yield 40).
    #[test]
    fn stacked_after_installs_thread_the_result_in_reverse_application_order() {
        let src = r#"
fn add_ten(x: int) -> int { return x + 10 }
fn mul_two(x: int) -> int { return x * 2 }

annotation first_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(after_hook(add_ten, []))
  }
}

annotation second_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(after_hook(mul_two, []))
  }
}

@first_hook()
@second_hook()
fn victim(a: int) -> int { return a * 10 }

victim(1)
"#;
        let (value, _) = top_level_i64(src);
        assert_eq!(
            value, 30,
            "after chain in REVERSE application order (onion; application order ⇒ 40)"
        );
    }

    // ── replace body + install on ONE target ───────────────────────────────

    // The weave wraps the FINAL (replace-body-EDITED) definition: the
    // replacement (`ctx.original(a) + 7`) becomes the shadow, and the
    // before hook mutates the arg feeding it: victim(4) → before 4+1=5 →
    // edited impl: original(5)+7 = 50+7 = 57. Un-edited would be 50;
    // un-hooked would be 47.
    #[test]
    fn weave_wraps_the_replace_body_edited_definition() {
        let src = r#"
fn add_one(x: int) -> int { return x + 1 }

annotation edit_and_hook() {
  targets: [function]
  comptime post(target, ctx) {
    replace body { return ctx.original(a) + 7 }
    install(before_hook(add_one, []))
  }
}

@edit_and_hook()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
        let (value, _) = top_level_i64(src);
        assert_eq!(
            value, 57,
            "the weave must wrap the EDITED body: before(4)=5 → original(5)+7 = 57"
        );
    }

    // ── the alias-spelling regression (S1 observation, stage item 3) ───────

    // Two targets spell the SAME concrete Sig differently (`Array<int>` /
    // `Vec<int>` both resolve to ConcreteType::Array(I64)): the handler is
    // shared via the injective Sig cache key, and BOTH weaves compile —
    // the weave consumes carrier annotations via ConcreteType equality,
    // never spelling equality.
    #[test]
    fn alias_spelled_same_concrete_targets_share_a_handler_and_both_weave() {
        let src = r#"
fn tmpl<Args>(args: Args) -> Args {
    args[1] = args[1] + 1
    return args
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, []))
  }
}

@hookann()
fn victim_a(xs: Array<int>, n: int) -> int { return n }

@hookann()
fn victim_b(xs: Vec<int>, n: int) -> int { return n * 100 }

victim_a([1], 4) * 10000 + victim_b([2], 7)
"#;
        let (value, compiler) = top_level_i64(src);
        // victim_a: n=4+1=5; victim_b: n=7+1=8 → 800.
        assert_eq!(value, 50800, "both alias-spelled weaves compile and execute");
        assert_eq!(compiler.hook_install_registry.len(), 2);
        assert_eq!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "one ConcreteType Sig ⇒ one shared handler across alias spellings"
        );
    }

    // ── the aggregate-kind guard (closing the S1 pending observation) ──────

    // A PROVEN-divergent carrier write (`args[0] = "boom"` into a
    // declared-int slot — the S1 measured near-miss) is a NAMED rejection
    // wrapped with BOTH signatures at the application site: never a
    // silently kind-divergent typed read in the weave.
    #[test]
    fn kind_divergent_carrier_write_is_a_named_rejection() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   args[0] = \"boom\"\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        let text = result
            .expect_err("a kind-divergent carrier write must reject at specialization")
            .to_string();
        assert!(
            text.contains("proves type `string`") && text.contains("declares `int`"),
            "names BOTH the proven and the declared kind: {text}"
        );
        assert!(
            text.contains("assign a `int` value"),
            "carries the positive twin: {text}"
        );
        assert!(
            text.contains("<Args>(args: Args) -> Args")
                && text.contains("(int, number) -> (int, number)"),
            "wrapped with both signatures at the application site: {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // An UNPROVABLE carrier write (a method call on a pseudo-slot — the
    // other S1 near-miss shape) rejects surface-and-stop with the
    // provable-domain positive twin.
    #[test]
    fn unprovable_carrier_write_is_a_named_rejection() {
        let (result, _) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   args[0] = args[0].trim()\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        let text = result
            .expect_err("an unprovable carrier write must reject at specialization")
            .to_string();
        assert!(
            text.contains("cannot prove the type of the value assigned to `args[0]`"),
            "names the unprovable write: {text}"
        );
        assert!(
            text.contains("provable at specialization"),
            "carries the provable-domain positive twin: {text}"
        );
    }

    // ── S6 supervisor-ordered guard growth: provable-initializer locals ────

    // The A-phase finding-1 shape: an exchange has no temp-free spelling, so
    // a hoisted local (`let t = args[0]`) must join the provable write-RHS
    // set at its binding. victim(3, 10) with swapped args = sub(10, 3) = 7
    // (an unswapped run yields -7 — value-distinguishing).
    #[test]
    fn hoisted_local_exchange_write_is_provable_and_weaves() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   let t = args[0]\n\
             \x20   args[0] = args[1]\n\
             \x20   args[1] = t\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a - b }\n\nvictim(3, 10)",
        ));
        assert_eq!(value, 7, "the exchange must be observed (unswapped ⇒ -7)");
        assert_eq!(compiler.hook_install_registry.len(), 1);
    }

    // Transitivity: a local bound to arithmetic over another provable local
    // is itself provable.
    #[test]
    fn transitively_provable_local_write_weaves() {
        let (value, _) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   let t = args[0]\n\
             \x20   let u = t + 1\n\
             \x20   args[0] = u\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 50, "the transitive local mutation must be observed (skip ⇒ 40)");
    }

    // An UNPROVABLE initializer keeps the local outside the provable set —
    // the existing named sentence stands (the ordered fix's boundary).
    #[test]
    fn unprovable_initializer_local_write_still_rejects() {
        let (result, _) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   let t = args[0].trim()\n\
             \x20   args[0] = t\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        let text = result
            .expect_err("an unprovable-initializer local write must reject at specialization")
            .to_string();
        assert!(
            text.contains("cannot prove the type of the value assigned to `args[0]`"),
            "names the unprovable write: {text}"
        );
    }

    // A local that is ASSIGNED after binding never joins the provable set
    // (a textual walk cannot bound loop re-execution order, so mutation is
    // conservative poison — sugar-minted bodies have no per-occurrence
    // inference facts to fall back to).
    #[test]
    fn reassigned_local_stays_outside_the_provable_write_set() {
        let (result, _) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   let mut t = args[0]\n\
             \x20   t = args[0].trim()\n\
             \x20   args[0] = t\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        let text = result
            .expect_err("a mutated local must stay outside the provable write set")
            .to_string();
        assert!(
            text.contains("cannot prove the type of the value assigned to `args[0]`"),
            "names the unprovable write: {text}"
        );
    }

    // ── S6 soundness fixlet: the AFTER-side return-kind gate ───────────────

    // The measured A-phase heap-pointer leak, API-path spelling: a
    // polymorphic `after` body whose actual return is a string, installed on
    // an int-returning target, previously specialized and RAN — printing the
    // string's heap pointer as the int result. The gate makes it the
    // established two-signature application-site rejection (the S2c analog).
    // The capture rides the with-captures route through
    // `ensure_monomorphic_template_specialization`; the gate fires BEFORE
    // the ride on both routes.
    #[test]
    fn after_template_type_changing_body_is_a_named_rejection() {
        let (result, compiler) = compile_source(&hook_source(
            "fn stringy<R>(result: R, tag: string) -> R { return f\"{tag}: {result}\" }",
            "let t: string = \"pfx\"\n    install(after_hook(stringy, [capture(\"tag\", t)]))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a + b }\n\nvictim(3, 4)",
        ));
        let text = result
            .expect_err("a type-changing after body must reject at specialization")
            .to_string();
        assert!(
            text.contains("the template body returns `string`")
                && text.contains("returns `int` (the target's declared result type)"),
            "names the proven and the required result kind: {text}"
        );
        assert!(
            text.contains("<R>(result: R) -> R") && text.contains("(int) -> int"),
            "wrapped with both signatures at the application site: {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // Zero-capture route twin of the rejection above (rides the plain
    // `ensure_monomorphic_function_for_callsite` path).
    #[test]
    fn after_template_type_changing_body_rejects_on_the_zero_capture_route() {
        let (result, _) = compile_source(&hook_source(
            "fn stringy<R>(result: R) -> R { return \"xx\" }",
            "install(after_hook(stringy, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        let text = result
            .expect_err("the bare-string after body must reject at specialization")
            .to_string();
        assert!(
            text.contains("the template body returns `string`"),
            "names the proven kind: {text}"
        );
    }

    // Positive twin: a bound-honoring `(R) -> R` body (branchy, with a
    // provable local) still specializes, weaves, and threads the result.
    #[test]
    fn after_template_return_gate_positive_twin_specializes_and_runs() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn post<R>(result: R) -> R {\n\
             \x20   let keep = result\n\
             \x20   if true { return keep }\n\
             \x20   return result\n\
             }",
            "install(after_hook(post, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 40, "the bound-honoring after body threads the result");
        assert_eq!(compiler.hook_install_registry.len(), 1);
    }

    // A body that can complete WITHOUT a value (`(R) -> R` declared, no
    // value-producing exit) never leaks unit bits as `R`. On the
    // analyzer-visited API path the DEFINITION-time check fires first
    // ("must return a value" — pinned here); the gate's own
    // `can complete without returning a value` arm backstops the
    // never-analyzer-visited sugar path (pinned in
    // `tools/shape-test/tests/annotations_runtime/wrapping.rs`).
    #[test]
    fn after_template_body_without_a_return_value_is_a_named_rejection() {
        let (result, _) = compile_source(&hook_source(
            "fn observer_ish<R>(result: R) -> R { let x = result }",
            "install(after_hook(observer_ish, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        let text = result
            .expect_err("a value-less after body must reject before any weave")
            .to_string();
        assert!(
            text.contains("must return a value"),
            "the definition-time value-less rejection fires: {text}"
        );
    }

    // The CONCRETE after twin is covered UPSTREAM (verified, pinned here):
    // a concrete template body is an ordinary analyzer-visited module fn, so
    // a body-vs-declared-return lie dies at DEFINITION with the constraint
    // solver's sentence — before any install/specialization runs. (The
    // declared-vs-required match at the application site is the existing
    // `require_specialization_position_match` pin family.)
    #[test]
    fn concrete_after_body_return_lie_rejects_at_definition() {
        let (result, _) = compile_source(&hook_source(
            "fn bad(result: int) -> int { return \"xx\" }",
            "install(after_hook(bad, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        let text = result
            .expect_err("a concrete body return lie must reject at definition compile")
            .to_string();
        assert!(
            text.contains("is not compatible with"),
            "the analyzer's constraint sentence fires: {text}"
        );
    }

    // ── S6 fixlet round 2: the `?`-exit arm (F1) + body-level-only returns
    // ── (F2), API-path twins ───────────────────────────────────────────────

    // F1, after side, API path — covered UPSTREAM (verified, pinned): an
    // API-path template body is an ordinary analyzer-visited module fn, so
    // a body-level `?` in a `<R>(result: R) -> R` body dies at DEFINITION
    // with the analyzer's Result/Option constraint — before any install or
    // specialization runs (the same layering as the value-less-body pin
    // above). The scan's own `?`-exit arm backstops the never-analyzer-
    // visited sugar path, where the round-2 probe MEASURED the Err carrier
    // escaping as `R` (`add(3, 4) + 1` printed the pointer bits
    // `102997035238305`) — pinned in
    // `tools/shape-test/tests/annotations_runtime/wrapping.rs`.
    #[test]
    fn after_template_try_operator_exit_is_a_named_rejection() {
        let (result, compiler) = compile_source(&hook_source(
            "fn fallible(flag: int) -> Result<int, string> {\n\
             \x20   if flag == 1 { return Ok(5) }\n\
             \x20   return Err(\"boom\")\n\
             }\n\
             fn post<R>(result: R) -> R {\n\
             \x20   let x = fallible(0)?\n\
             \x20   return result\n\
             }",
            "install(after_hook(post, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        let text = result
            .expect_err("a body-level `?` in an after template body must reject")
            .to_string();
        assert!(
            text.contains("operator '?' requires the function to return Result or Option"),
            "the definition-time Result/Option constraint fires first on the API path: {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected program leaves no registry row"
        );
    }

    // F1, before side, API path — the same upstream definition-time
    // coverage for a `<Args>(args: Args) -> Args` body with a body-level
    // `?`; the scan arm's before-side backstop (the sugar path, where the
    // probe MEASURED silent corruption of the woven call: `1` where `8`)
    // is pinned in `tools/shape-test/tests/annotations_runtime/injection.rs`
    // and on the rewrite face directly in `pseudo_tuple.rs`.
    #[test]
    fn before_template_try_operator_exit_is_a_named_rejection() {
        let (result, _) = compile_source(&hook_source(
            "fn fallible(flag: int) -> Result<int, string> {\n\
             \x20   if flag == 1 { return Ok(5) }\n\
             \x20   return Err(\"boom\")\n\
             }\n\
             fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   let x = fallible(0)?\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a - b }\n\nvictim(3, 10)",
        ));
        let text = result
            .expect_err("a body-level `?` in a before template body must reject")
            .to_string();
        assert!(
            text.contains("operator '?' requires the function to return Result or Option"),
            "the definition-time Result/Option constraint fires first on the API path: {text}"
        );
    }

    // F2, fixed case (API path): a closure helper's own internal return is
    // NOT a template-body exit — before the filter, its non-`R` return
    // false-positively rejected this type-correct after body (MEASURED
    // round-2 probe on the sugar twin). The `?` inside a closure is likewise
    // closure-frame-local (pinned on the rewrite face in pseudo_tuple.rs).
    #[test]
    fn after_template_closure_internal_return_stays_green() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn post<R>(result: R) -> R {\n\
             \x20   let f = |x: int| { return \"s\" }\n\
             \x20   let s = f(1)\n\
             \x20   return result\n\
             }",
            "install(after_hook(post, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(
            value, 40,
            "the closure's internal return must not enter the return-kind guard"
        );
        assert_eq!(compiler.hook_install_registry.len(), 1);
    }

    // ── S6 fixlet round 3 (F3): the BEFORE-side exit gate, API-path twins ──

    // MUST-REJECT (stray value exit): a polymorphic before body whose exit
    // delivers a plain int on a 2-ary target can never be the
    // compiler-internal argument aggregate the weave reads back per-field —
    // pre-gate this specialized and wove, reaching the woven typed local's
    // read unchecked (the round-1-lens F3 finding; the sugar twin is pinned
    // in `tools/shape-test/tests/annotations_runtime/injection.rs`). The
    // analyzer does NOT check a generic body's return against `Args` at
    // definition (the measured after-side symmetry), so the gate is the live
    // line on BOTH paths for this shape.
    #[test]
    fn before_template_stray_value_exit_is_a_named_rejection() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\
             \x20   args[0] = args[0] + 1\n\
             \x20   return 42\n\
             }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a + b }\n\nvictim(3, 4)",
        ));
        let text = result
            .expect_err("a stray-value before exit must reject at specialization")
            .to_string();
        assert!(
            text.contains("delivers `int` at an exit")
                && text.contains("compiler-internal argument aggregate over (a: int, b: int)"),
            "names the proven kind and the bound carrier: {text}"
        );
        assert!(
            text.contains("<Args>(args: Args) -> Args")
                && text.contains("(int, int) -> (int, int)"),
            "wrapped with both signatures at the application site: {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // MUST-REJECT (value-less body): covered UPSTREAM on the analyzer-visited
    // API path (verified, pinned AS-IS — the round-1 layering convention): a
    // `<Args>(args: Args) -> Args` body with no value-producing exit dies at
    // DEFINITION — the inference unifies the value-less body with
    // `Args := Void`, so the body's `args[0]` read violates the Void
    // index-access constraint (MEASURED sentence; upstream's wording, not
    // this gate's charter). The gate's own pack-less arm backstops the
    // never-analyzer-visited sugar path (pinned in injection.rs) and the
    // branch-fall-through shape (pinned on the rewrite face in
    // pseudo_tuple.rs).
    #[test]
    fn before_template_value_less_body_is_a_named_rejection() {
        let (result, _) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args { let x = args[0] }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a + b }\n\nvictim(3, 4)",
        ));
        let text = result
            .expect_err("a value-less before body must reject before any weave")
            .to_string();
        assert!(
            text.contains("Concrete(Void) does not support index access"),
            "the definition-time constraint rejection fires on the API path: {text}"
        );
    }

    // MUST-REJECT (value-less body, args untouched): the variant with no
    // `args` read dies at DEFINITION with the same "must return a value"
    // check as the after side (MEASURED — pinned as-is). Between this pin
    // and the one above, BOTH value-less API spellings are upstream-covered;
    // the gate's pack-less arm's live line is the never-analyzer-visited
    // sugar path plus the branch-fall-through shapes the definition check
    // cannot see (missing-else / bare-`return` — pinned on the rewrite face
    // in pseudo_tuple.rs).
    #[test]
    fn before_template_value_less_args_untouched_body_rejects_at_definition() {
        let (result, _) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args { let x = 1 }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: int) -> int { return a + b }\n\nvictim(3, 4)",
        ));
        let text = result
            .expect_err("an args-untouched value-less before body must reject")
            .to_string();
        assert!(
            text.contains("must return a value"),
            "the definition-time value-less rejection fires: {text}"
        );
    }

    // POSITIVE TWIN (type-proof, end-to-end): a NON-canonical exit proving
    // the Single carrier type (`return args[0] * 2` on a 1-ary int target)
    // specializes, weaves, and delivers the doubled argument — the gate
    // proves types, never polices spellings (the shape was type-sound and
    // working pre-gate; gated to exactly what the weave accepts).
    #[test]
    fn before_template_conforming_value_exit_specializes_and_runs() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args { return args[0] * 2 }",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 80, "the carrier-proving exit delivers the doubled arg");
        assert_eq!(compiler.hook_install_registry.len(), 1);
    }

    // ── a void target hosts a before-only weave ────────────────────────────

    #[test]
    fn void_target_with_before_hook_weaves_and_executes() {
        let compiler = compiled_ok(&hook_source(
            "fn add_one(x: int) -> int { return x + 1 }",
            "install(before_hook(add_one, []))",
            "@hookann()\nfn log_it(a: int) { let x = a }\n\nlog_it(4)\n7",
        ));
        let value = execute_top_level(&compiler)
            .as_i64()
            .expect("trailing literal is the top-level value");
        assert_eq!(value, 7, "the void weave executes without corrupting the stack");
        assert_eq!(shadow_entries(&compiler, "log_it").len(), 1);
    }

    // ── an async target: the wrapper awaits the async shadow ───────────────

    #[test]
    fn async_target_weave_awaits_the_shadow_and_observes_the_mutation() {
        let (value, _) = top_level_i64(&hook_source(
            "fn add_one(x: int) -> int { return x + 1 }",
            "install(before_hook(add_one, []))",
            "@hookann()\nasync fn victim(a: int) -> int { return a * 10 }\n\nawait victim(4)",
        ));
        assert_eq!(value, 50, "the async weave awaits the shadow (skip ⇒ 40)");
    }

    // ── the OBSERVER form (fix-round-1 C3-G2 growth) ───────────────────────

    // GREEN CONTROL: before+after observers on a ZERO-PARAM VOID target (the
    // canonical entry/exit-logging shape — the S1c/S2 hole: this target
    // previously could receive NO hook at all) weave and execute without
    // corrupting the top level. Structural pins: shadow + wrapper + 2 rows.
    #[test]
    fn observers_on_a_zero_param_void_target_weave_and_execute() {
        let compiler = compiled_ok(&hook_source(
            "fn note_in() { let x = 1 }\nfn note_out() { let x = 2 }",
            "install(before_hook(note_in, []))\n    install(after_hook(note_out, []))",
            "@hookann()\nfn hello() { let a = 1 }\n\nhello()\n7",
        ));
        let value = execute_top_level(&compiler)
            .as_i64()
            .expect("trailing literal is the top-level value");
        assert_eq!(value, 7, "the observer weave executes cleanly (green control)");
        assert_eq!(shadow_entries(&compiler, "hello").len(), 1);
        assert_eq!(compiler.hook_install_registry.len(), 2);
        assert_eq!(
            compiler.hook_install_registry[0].hook_kind,
            TemplateHookKind::Before
        );
        assert_eq!(
            compiler.hook_install_registry[1].hook_kind,
            TemplateHookKind::After
        );
    }

    // EXECUTION PROOF (the observer has no data-flow observable, so the
    // proof is error-injection): an observer whose body errors at runtime
    // makes the WHOLE woven program error — iff the observer actually runs.
    // The green control above is the non-vacuity twin (same weave shape,
    // non-erroring observer, program runs green).
    #[test]
    fn observer_execution_is_proven_by_an_erroring_observer_body() {
        for (hook_stmt, kind) in [
            ("install(before_hook(boom, []))", "before"),
            ("install(after_hook(boom, []))", "after"),
        ] {
            let compiler = compiled_ok(&hook_source(
                "fn boom() {\n\
                 \x20   let xs = [1, 2]\n\
                 \x20   let mut i = 0\n\
                 \x20   while i < 9 { i = i + 1 }\n\
                 \x20   let y = xs[i]\n\
                 }",
                hook_stmt,
                "@hookann()\nfn hello() { let a = 1 }\n\nhello()\n7",
            ));
            let mut vm = VirtualMachine::new(VMConfig::default());
            vm.load_program(compiler.program.clone());
            vm.execute(None).expect_err(&format!(
                "the woven {kind} observer must EXECUTE (its body errors at runtime)"
            ));
        }
    }

    // Observers are TARGET-UNIFORM and compose with mutation hooks: a before
    // observer beside a mutating before on a param-bearing target leaves the
    // args chain untouched — (4+1)*10 = 50 exactly as without the observer.
    #[test]
    fn observer_composes_with_mutation_hooks_without_touching_the_chain() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn note() { let x = 1 }\n\
             fn add_one(x: int) -> int { return x + 1 }",
            "install(before_hook(note, []))\n    install(before_hook(add_one, []))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(
            value, 50,
            "the observer must not disturb the mutation chain (skip-mutation ⇒ 40)"
        );
        assert_eq!(compiler.hook_install_registry.len(), 2);
    }

    // An observer with a CAPTURE (S3b re-target): the observer is called
    // with ZERO arguments — its capture value is BAKED into the observer
    // specialization's prologue — and the row records the literal.
    #[test]
    fn observer_with_capture_weaves_and_records_the_literal() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tagged(tag: int) { let x = tag }",
            "install(before_hook(tagged, [capture(\"tag\", 3)]))",
            "@hookann()\nfn hello() { let a = 1 }\n\nhello()\n7",
        ));
        assert_eq!(value, 7);
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![("tag".to_string(), "3".to_string())]
        );
    }

    // ── rollback: a failing later install leaves NO weave residue ──────────

    // The FIRST target weaves fully (shadow + wrapper + registry row); the
    // SECOND target's install fails at pass-2 (zero-param before) — the
    // whole compile rolls back: no registry row, no cache entry, and no
    // unspellable weave shadow survives in the function table. Non-vacuity:
    // the error names the SECOND target, and a fresh compile of the
    // un-annotated program preserves the target's ORIGINAL behavior.
    #[test]
    fn failing_later_install_rolls_back_shadow_wrapper_and_registry() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\x20   return args\n}",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\n\
             @hookann()\nfn zero() -> int { return 7 }\n\nvictim(1, 2.0)",
        ));
        let text = result
            .expect_err("the zero-param before target must fail the compile")
            .to_string();
        assert!(
            text.contains("zero") && text.contains("declares no parameters"),
            "the failure comes from the SECOND target (the first had already woven): {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "rollback removes the first target's registry row"
        );
        assert_eq!(
            compiler.monomorphization_cache.legacy_len(),
            0,
            "rollback evicts the specialization cache"
        );
        assert!(
            !compiler
                .program
                .functions
                .iter()
                .any(|function| function.name.starts_with('\u{1}')),
            "no hygienic weave shadow survives the rollback"
        );

        // Original behavior preserved: the plain program still runs 1*10.
        let (value, _) = top_level_i64(
            "fn victim(a: int) -> int { return a * 10 }\n\nvictim(1)",
        );
        assert_eq!(value, 10);
    }

    // ── S3b: composite config end-to-end (rule 6 + the bake) ───────────────
    //
    // #65 FENCE (every fixture below): NEVER spell an array literal INLINE
    // in the element expression of the TypedObject-element captures array —
    // `[capture("cfg", [1, 2])]` trips the PRE-EXISTING #65
    // `pending_variable_typed_array_kind` leak (runtime "expected
    // Ptr(TypedObject), got Int64"). ALWAYS hoist to a local first:
    // `let cfg = [1, 2]` then `install(before_hook(f, [capture("cfg",
    // cfg)]))`. #65 itself is pre-existing and NOT fixed in S3.

    // A CONCRETE before template with a capture through the full weave: the
    // handler is the BAKED suffixed specialization (arity == Sig arity); the
    // definition-compiled body fn remains an ordinary module fn beside it.
    #[test]
    fn concrete_install_with_capture_weaves_through_the_baked_specialization() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn add_n(x: int, n: int) -> int { return x + n }",
            "install(before_hook(add_n, [capture(\"n\", 7)]))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        assert_eq!(value, 110, "before(4) = 4 + baked 7 = 11 → impl 110 (skip ⇒ 40)");
        let row = &compiler.hook_install_registry[0];
        assert!(
            row.specialized_symbol.contains("::cfg#1::i:7"),
            "the concrete install resolves the BAKED suffixed specialization: {}",
            row.specialized_symbol
        );
        assert!(
            compiler.find_function("add_n").is_some(),
            "the definition-compiled body fn remains an ordinary module fn"
        );
    }

    // Rule-6 DISTINCT over composite config ([1,2] / [1,2,3] / [2,1]):
    // three annotations, structurally different Array<int> config, one
    // same-Sig target set ⇒ three DISTINCT baked specializations with
    // distinct observable behavior. (#65 fence: cfg hoisted to a local.)
    #[test]
    fn rule6_distinct_composite_config_gets_distinct_specializations() {
        let src = r#"
fn tmpl<Args>(args: Args, cfg: Array<int>) -> Args {
    args[0] = args[0] * 100 + cfg[0] * 10 + cfg.length()
    return args
}

annotation with_one_two() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [1, 2]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation with_one_two_three() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [1, 2, 3]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation with_two_one() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [2, 1]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

@with_one_two()
fn victim_a(a: int) -> int { return a }

@with_one_two_three()
fn victim_b(a: int) -> int { return a }

@with_two_one()
fn victim_c(a: int) -> int { return a }

victim_a(1) * 1000000 + victim_b(1) * 1000 + victim_c(1)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 100 + 1*10 + 2 = 112; b: 100 + 10 + 3 = 113; c: 100 + 20 + 2 = 122.
        assert_eq!(
            value, 112_113_122,
            "each baked specialization observes exactly its own composite config"
        );
        assert_eq!(compiler.hook_install_registry.len(), 3);
        let indices: Vec<u16> = compiler
            .hook_install_registry
            .iter()
            .map(|row| row.function_index)
            .collect();
        assert_ne!(indices[0], indices[1], "[1,2] vs [1,2,3] are distinct (rule 6)");
        assert_ne!(indices[0], indices[2], "[1,2] vs [2,1] are distinct (rule 6)");
        assert_ne!(indices[1], indices[2], "[1,2,3] vs [2,1] are distinct (rule 6)");
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![("cfg".to_string(), "[1, 2]".to_string())],
            "the registry renders the composite capture"
        );
    }

    // Rule-6 SHARE over composite config: two annotations with structurally
    // EQUAL Array<int> config on two same-Sig targets share ONE baked
    // specialization — and both targets execute correctly through it.
    // (#65 fence: cfg hoisted.)
    #[test]
    fn rule6_equal_composite_config_shares_one_specialization() {
        let src = r#"
fn tmpl<Args>(args: Args, cfg: Array<int>) -> Args {
    args[0] = args[0] * 100 + cfg[0] * 10 + cfg.length()
    return args
}

annotation first_config() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [1, 2]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation second_config() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [1, 2]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

@first_config()
fn victim_a(a: int) -> int { return a }

@second_config()
fn victim_b(a: int) -> int { return a + 1 }

victim_a(1) * 1000 + victim_b(2)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 112; b: 2*100+10+2 = 212 → +1 = 213.
        assert_eq!(value, 112_213, "both targets execute through the shared baked handler");
        assert_eq!(compiler.hook_install_registry.len(), 2);
        assert_eq!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "rule 6: structurally EQUAL composite config SHARES one specialization"
        );
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            compiler.hook_install_registry[1].captures,
            "non-vacuity: both rows record the same [1, 2] rendering"
        );
    }

    // KEY-LEVEL injectivity end-to-end: ("ab","c") vs ("a","bc") —
    // Array<string> config whose FLAT concatenation collides (the in-test
    // control) must produce DISTINCT baked specializations with distinct
    // observable behavior. (#65 fence: cfg hoisted.)
    #[test]
    fn rule6_string_redistribution_config_stays_distinct_end_to_end() {
        // Control (non-vacuity): the flat join of the two value lists
        // really collides.
        assert_eq!(
            ["ab", "c"].concat(),
            ["a", "bc"].concat(),
            "control: the flat join must collide for this refuter to bite"
        );
        let src = r#"
fn tmpl<Args>(args: Args, cfg: Array<string>) -> Args {
    args[0] = args[0] * 10 + cfg[0].length()
    return args
}

annotation with_ab_c() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = ["ab", "c"]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation with_a_bc() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = ["a", "bc"]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

@with_ab_c()
fn victim_a(a: int) -> int { return a }

@with_a_bc()
fn victim_b(a: int) -> int { return a }

victim_a(1) * 1000 + victim_b(1)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 10 + len("ab") = 12; b: 10 + len("a") = 11.
        assert_eq!(value, 12011, "each handler observes its own string boundaries");
        assert_ne!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "the netstring config segment must distinguish the redistributed pair"
        );
    }

    // NESTED composite config: [[1,2],[3]] vs [[1],[2,3]] — the flat-leaf
    // collision pair (count prefixes pin nesting boundaries) must produce
    // distinct baked specializations with distinct behavior. (#65 fence:
    // cfg hoisted; per the charter's probe protocol the STANDALONE nested
    // literal `let cfg = [[1, 2], [3]]` is probed first — if a sibling
    // stamp leak fires the fixture element-hoists AND the repro surfaces on
    // #65's thread.)
    #[test]
    fn rule6_nested_composite_config_stays_distinct_end_to_end() {
        let src = r#"
fn tmpl<Args>(args: Args, cfg: Array<Array<int>>) -> Args {
    args[0] = args[0] * 100 + cfg[0].length() * 10 + cfg.length()
    return args
}

annotation with_nested_a() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [[1, 2], [3]]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation with_nested_b() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [[1], [2, 3]]
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

@with_nested_a()
fn victim_a(a: int) -> int { return a }

@with_nested_b()
fn victim_b(a: int) -> int { return a }

victim_a(1) * 1000 + victim_b(1)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 100 + 2*10 + 2 = 122; b: 100 + 1*10 + 2 = 112.
        assert_eq!(value, 122_112, "each handler observes its own nesting boundaries");
        assert_ne!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "count-prefixed segments must distinguish the nested pair"
        );
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![("cfg".to_string(), "[[1, 2], [3]]".to_string())]
        );
    }

    // OPTION config: Some(5) vs None — distinct baked specializations,
    // distinct behavior, rows render the user spellings. (#65 fence: cfg
    // hoisted. The Some(5)-vs-bare-5 pair is a KEY-LEVEL refuter in mod.rs
    // — one template cannot declare a capture param typed both `int` and
    // `Option<int>`, so the API cannot spell that pair end-to-end.)
    #[test]
    fn rule6_option_config_some_vs_none_stays_distinct_end_to_end() {
        let src = r#"
fn tmpl<Args>(args: Args, cfg: Option<int>) -> Args {
    let bump = match cfg { Some(n) => n, None => 1 }
    args[0] = args[0] + bump
    return args
}

annotation with_some() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg: Option<int> = Some(5)
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

annotation with_none() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg: Option<int> = None
    install(before_hook(tmpl, [capture("cfg", cfg)]))
  }
}

@with_some()
fn victim_a(a: int) -> int { return a }

@with_none()
fn victim_b(a: int) -> int { return a }

victim_a(10) * 1000 + victim_b(10)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 10 + 5 = 15; b: 10 + 1 = 11.
        assert_eq!(value, 15011, "each handler observes its own Option config");
        assert_ne!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "Some(5) and None are structurally distinct (rule 6)"
        );
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![("cfg".to_string(), "Some(5)".to_string())]
        );
        assert_eq!(
            compiler.hook_install_registry[1].captures,
            vec![("cfg".to_string(), "None".to_string())]
        );
    }

    // EMPTY-ARRAY config PROBE RESULT (the charter's named contingency
    // point, S3b probe 2026-07-20): the handler-side spelling `let cfg:
    // Array<int> = []` cannot even CONSTRUCT the value — the comptime
    // mini-VM rejects the empty typed-array literal with the PRE-EXISTING
    // loud "[C0001] this operation is not available in compile-time code"
    // error BEFORE `capture()` is reached, so the charter's
    // validate_capture_value_types named-rejection contingency has no value
    // to fire on through the API today. This pin locks the LOUD
    // surface-and-stop behavior (never a silent crash); the BAKED-PROLOGUE
    // half of the probe is proven at the seam
    // (`mod.rs::empty_array_capture_value_bakes_at_the_seam` — the value is
    // host-constructible and bakes/executes green), so the residual is
    // exactly the pre-existing comptime empty-array-literal gap — disclosed
    // in the slice report for supervisor relay. (#65 fence: hoisted.)
    #[test]
    fn empty_array_config_probe_is_a_loud_comptime_rejection_today() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args, cfg: Array<int>) -> Args {\n\
             \x20   args[0] = args[0] * 10 + cfg.length()\n\
             \x20   return args\n\
             }",
            "let cfg: Array<int> = []\n    install(before_hook(tmpl, [capture(\"cfg\", cfg)]))",
            "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
        ));
        let text = result
            .expect_err("the empty comptime array literal is a loud pre-existing rejection")
            .to_string();
        assert!(
            text.contains("not available in compile-time code"),
            "the pre-existing loud comptime rejection fires (never a silent crash): {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "no install lands behind the rejection"
        );
    }

    // ── S3c: the W39/LoadModuleBinding named check (charter (e)) ───────────
    //
    // The S0 §4 a4d/a4e-noted JIT poison: the deleted LEGACY config path
    // read its config from a module binding at every invocation, planting
    // `LoadModuleBinding` (0x52) in the generated wrapper ("generated
    // wrapper contains `LoadModuleBinding` → W39 whole-program deopt"; the
    // S6 capstone deleted that machinery).
    // The NEW path bakes capture values as CONSTANTS at specialization, so
    // config access must emit ZERO module-binding loads in the specialized
    // handler AND the generated wrapper. This is a BYTECODE-level pin — it
    // holds regardless of JIT status; the CLI zero-fallback twin
    // (`jit_c3_carrier_native.rs::c3_composite_config_single_runs_natively_
    // both_tiers`) proves the same shape natively.

    /// The full module-binding LOAD family: the S0-named legacy
    /// `LoadModuleBinding` (0x52) plus the typed Wave-E+3 load variants
    /// (0x182..=0x18C). The typed variants are emitter-dead at HEAD
    /// (`typed_load_module_binding_opcode` is `#[allow(dead_code)]`), but
    /// the scan covers them anyway so a future emitter flip cannot
    /// reintroduce config-via-module-binding under a typed spelling without
    /// failing this pin.
    fn is_module_binding_load(opcode: crate::bytecode::OpCode) -> bool {
        use crate::bytecode::OpCode as Op;
        matches!(
            opcode,
            Op::LoadModuleBinding
                | Op::LoadModuleBindingI64
                | Op::LoadModuleBindingU64
                | Op::LoadModuleBindingF64
                | Op::LoadModuleBindingI32
                | Op::LoadModuleBindingU32
                | Op::LoadModuleBindingI16
                | Op::LoadModuleBindingU16
                | Op::LoadModuleBindingI8
                | Op::LoadModuleBindingU8
                | Op::LoadModuleBindingBool
                | Op::LoadModuleBindingPtr
        )
    }

    /// Count module-binding loads in one registered function's body slice.
    fn module_binding_loads(
        compiler: &crate::compiler::BytecodeCompiler,
        function: &crate::bytecode::Function,
    ) -> usize {
        let end = function.entry_point + function.body_length;
        compiler.program.instructions[function.entry_point..end]
            .iter()
            .filter(|instr| is_module_binding_load(instr.opcode))
            .count()
    }

    // Scalar + composite captures in one WOVEN PROGRAM — via TWO
    // annotations (one handler each). HISTORICAL NOTE (S4a): this fixture
    // was written under the S3-era handler-wide capture-type unification
    // limitation (all `capture()` value args in one handler had to agree);
    // S4a's generic `capture<T>` forwarder REMOVED that limitation (the
    // mixed-in-one-handler pins below), but this green pin keeps its
    // two-annotation fixture as-is — it independently proves both bake
    // spellings through one wrapper. Both bake spellings are exercised;
    // the executed value proves the baked config drives the mutation. (#65
    // fence: cfg hoisted to a handler local. The hygienic impl shadow is
    // OUT of this claim — it is USER code and may legitimately read module
    // bindings; the claim is about the GENERATED artifacts' config access.)
    #[test]
    fn baked_config_emits_no_module_binding_loads_in_handler_or_wrapper() {
        let src = r#"
fn tmpl_scalar<Args>(args: Args, bump: int) -> Args {
    args[0] = args[0] * bump
    return args
}
fn tmpl_array<Args>(args: Args, cfg: Array<int>) -> Args {
    args[0] = args[0] + cfg[0] * 10 + cfg.length()
    return args
}

annotation scalar_ann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl_scalar, [capture("bump", 3)]))
  }
}

annotation array_ann() {
  targets: [function]
  comptime post(target, ctx) {
    let cfg = [5, 6]
    install(before_hook(tmpl_array, [capture("cfg", cfg)]))
  }
}

@scalar_ann()
@array_ann()
fn victim(a: int) -> int { return a }

victim(4)
"#;
        let (value, compiler) = top_level_i64(src);
        // before chain in application order: 4*3 = 12 → 12 + 5*10 + 2 = 64
        // → impl(64) = 64. Skip-either ⇒ 16 / 12; any misread of either
        // baked constant shifts the value.
        assert_eq!(value, 64, "the baked scalar AND composite config drive the mutation");

        assert_eq!(compiler.hook_install_registry.len(), 2, "two config-bearing installs");
        for row in &compiler.hook_install_registry {
            let handler = &compiler.program.functions[usize::from(row.function_index)];
            assert!(
                handler.name.contains("::cfg#1"),
                "sanity: the handler IS a config-suffixed baked specialization: {}",
                handler.name
            );
            assert_eq!(
                module_binding_loads(&compiler, handler),
                0,
                "W39 named check: the specialized handler `{}` reads config from BAKED \
                 constants — zero module-binding loads (the S0 §4 legacy poison is absent \
                 from the new path)",
                handler.name
            );
        }

        let wrapper = function_entry(&compiler, "victim");
        assert_eq!(
            module_binding_loads(&compiler, wrapper),
            0,
            "W39 named check: the generated wrapper carries no per-invocation config \
             machinery — zero module-binding loads"
        );
    }

    // ── S4a (#66 item 1): per-call-site capture value typing ───────────────
    //
    // S4a PIN FLIP (the S4-opening fix, #66 item 1): the S3c probe pinned
    // capture VALUE types unifying HANDLER-WIDE (`[C0001] Could not solve
    // type constraints` — one shared inference var on the monomorphic
    // `capture` forwarder's value param). The forwarder is now the ONE
    // GENERIC forwarder — `capture<T>(name, value: T) -> __CaptureBinding`
    // (`comptime.rs::comptime_builtin_forwarders`, the named special-case)
    // — so each `capture()` call instantiates its own T through ordinary
    // generic-call inference + monomorphization, and the S3c fixture (int +
    // Array<int> in ONE captures array) compiles AND EXECUTES end-to-end.
    // Mixed-typed config is exactly the shape the S4 declarative sugar's
    // typed config params lower onto (C3-G2: the public API must carry it).
    #[test]
    fn mixed_capture_value_types_in_one_handler_execute_end_to_end() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args, bump: int, cfg: Array<int>) -> Args {\n\
             \x20   args[0] = args[0] * bump + cfg[0]\n\
             \x20   return args\n\
             }",
            "let cfg = [5, 6]\n    \
             install(before_hook(tmpl, [capture(\"bump\", 3), capture(\"cfg\", cfg)]))",
            "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(4)",
        ));
        // before: 4*3 + 5 = 17 → impl(17) = 17. Skip ⇒ 4; a misread of
        // either baked constant shifts the value.
        assert_eq!(value, 17, "BOTH mixed-typed baked constants drive the mutation");
        assert_eq!(compiler.hook_install_registry.len(), 1, "one install lands");
        let row = &compiler.hook_install_registry[0];
        assert_eq!(
            row.captures,
            vec![
                ("bump".to_string(), "3".to_string()),
                ("cfg".to_string(), "[5, 6]".to_string())
            ],
            "the row renders both capture values in delivery order"
        );
        assert!(
            row.specialized_symbol.contains("::cfg#2"),
            "the specialized symbol carries the two-value config arity head: {}",
            row.specialized_symbol
        );
    }

    // Charter pin (S4a): int + string captures in ONE handler — the exact
    // `annotation retry(times: int, label: string)` mixed-config shape the
    // S4 sugar lowers onto. Value-distinguishing: the string's length
    // enters the arithmetic, so a dropped or altered string capture shifts
    // the executed value.
    #[test]
    fn int_and_string_captures_in_one_handler_execute_end_to_end() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn tmpl<Args>(args: Args, bump: int, tag: string) -> Args {\n\
             \x20   args[0] = args[0] * bump + tag.length()\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, [capture(\"bump\", 3), capture(\"tag\", \"ab\")]))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        // before: 4*3 + len("ab") = 14 → impl(14) = 140. Skip ⇒ 40; a
        // dropped tag ⇒ 120; a different tag length shifts the value.
        assert_eq!(value, 140, "the int AND the string baked constants drive the mutation");
        assert_eq!(compiler.hook_install_registry.len(), 1, "one install lands");
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![
                ("bump".to_string(), "3".to_string()),
                ("tag".to_string(), "\"ab\"".to_string())
            ],
            "the row renders the int and the quoted string in delivery order"
        );
    }

    // Mixed capture types across TWO separate installs in ONE handler run
    // (the second spelling the S3c probe measured failing): a before with
    // an int capture + an after with an Array<int> capture, both installed
    // by the same handler, executed end-to-end.
    #[test]
    fn mixed_captures_across_two_installs_in_one_handler_execute_end_to_end() {
        let (value, compiler) = top_level_i64(&hook_source(
            "fn bump_by<Args>(args: Args, bump: int) -> Args {\n\
             \x20   args[0] = args[0] + bump\n\
             \x20   return args\n\
             }\n\
             fn scale_result(r: int, cfg: Array<int>) -> int {\n\
             \x20   return r * cfg[0] + cfg.length()\n\
             }",
            "let cfg = [5, 6]\n    \
             install(before_hook(bump_by, [capture(\"bump\", 3)]))\n    \
             install(after_hook(scale_result, [capture(\"cfg\", cfg)]))",
            "@hookann()\nfn victim(a: int) -> int { return a * 10 }\n\nvictim(4)",
        ));
        // before: 4+3 = 7 → impl(7) = 70 → after: 70*5 + 2 = 352. Skipping
        // the before ⇒ 202; skipping the after ⇒ 70.
        assert_eq!(value, 352, "the int before-capture AND the array after-capture both bake");
        assert_eq!(compiler.hook_install_registry.len(), 2, "both installs land");
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![("bump".to_string(), "3".to_string())]
        );
        assert_eq!(
            compiler.hook_install_registry[1].captures,
            vec![("cfg".to_string(), "[5, 6]".to_string())]
        );
    }

    // Rule-6 identity on MIXED config (S4a): two applications with equal
    // (int, string) config SHARE one baked specialization; a differing
    // string SPLITS — the structural spec-hash covers the WHOLE mixed
    // config vector, not just its first value.
    #[test]
    fn rule6_mixed_config_equal_shares_and_differing_splits() {
        let src = r#"
fn tmpl<Args>(args: Args, bump: int, tag: string) -> Args {
    args[0] = args[0] * bump + tag.length()
    return args
}

annotation ab_one() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("bump", 3), capture("tag", "ab")]))
  }
}

annotation ab_two() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("bump", 3), capture("tag", "ab")]))
  }
}

annotation with_xyz() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("bump", 3), capture("tag", "xyz")]))
  }
}

@ab_one()
fn victim_a(a: int) -> int { return a + 1 }

@ab_two()
fn victim_b(a: int) -> int { return a + 2 }

@with_xyz()
fn victim_c(a: int) -> int { return a + 3 }

(victim_a(10) * 1000 + victim_b(20)) * 1000 + victim_c(30)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: 10*3+2 = 32 → 33; b: 20*3+2 = 62 → 64; c: 30*3+3 = 93 → 96.
        assert_eq!(value, 33064096, "all three targets execute their own mixed config");
        assert_eq!(compiler.hook_install_registry.len(), 3);
        assert_eq!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "rule 6: structurally EQUAL (int, string) config SHARES one specialization"
        );
        assert_ne!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[2].function_index,
            "rule 6: a differing string in the mixed config vector SPLITS"
        );
    }

    // ═══ ADR-009 C3 #14 slice 4 (S4b): typed config params — the G2 e2e
    // proof BEFORE the sugar lowering ═══════════════════════════════════════

    // `annotation retry(times: int, tag: string)` with a HAND-WRITTEN
    // comptime post handler spelling the lowering through ONLY the public
    // API (install / before_hook / capture). The typed config params are
    // injected into the handler as ordinary TYPED params (slice-4 typed
    // injection); their VALUES arrive from each `@retry(...)` application's
    // args — two applications with DIFFERENT config prove the flow is real
    // (not a baked constant), and the mixed (int, string) captures ride the
    // S4a per-call-site typing. This is the C3-G2 API-completeness half:
    // the S4c sugar will lower onto a capability the API HAS — zero private
    // side-channels.
    #[test]
    fn typed_config_params_flow_through_the_public_api_end_to_end() {
        let src = r#"
fn bump<Args>(args: Args, times: int, tag: string) -> Args {
    args[0] = args[0] * times + tag.length()
    return args
}

annotation retry(times: int, tag: string) {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(bump, [capture("times", times), capture("tag", tag)]))
  }
}

@retry(3, "ab")
fn victim_a(a: int) -> int { return a * 10 }

@retry(5, "wxyz")
fn victim_b(a: int) -> int { return a * 10 }

victim_a(4) * 1000 + victim_b(4)
"#;
        let (value, compiler) = top_level_i64(src);
        // a: before 4*3 + len("ab") = 14 → impl(14) = 140.
        // b: before 4*5 + len("wxyz") = 24 → impl(24) = 240.
        // Skip ⇒ 40; swapped configs ⇒ 240140; a dropped tag shifts both.
        assert_eq!(
            value, 140_240,
            "each application's OWN typed config values drive its mutation"
        );
        assert_eq!(compiler.hook_install_registry.len(), 2, "both installs land");
        assert_eq!(
            compiler.hook_install_registry[0].captures,
            vec![
                ("times".to_string(), "3".to_string()),
                ("tag".to_string(), "\"ab\"".to_string())
            ],
            "row a renders the first application's config"
        );
        assert_eq!(
            compiler.hook_install_registry[1].captures,
            vec![
                ("times".to_string(), "5".to_string()),
                ("tag".to_string(), "\"wxyz\"".to_string())
            ],
            "row b renders the second application's config"
        );
        assert_ne!(
            compiler.hook_install_registry[0].function_index,
            compiler.hook_install_registry[1].function_index,
            "rule 6: differing typed config splits the baked specializations"
        );
    }

    // Mismatched config arg: `@retry("x", "y")` feeds a string application
    // arg to the injected `times: int` param — a LOUD compile-time
    // rejection (contains-level per the S4 charter; exact attribution
    // refinement is S5's). The green twin is the pin above.
    #[test]
    fn typed_config_mismatched_application_arg_is_a_loud_rejection() {
        let src = r#"
fn bump<Args>(args: Args, times: int, tag: string) -> Args {
    args[0] = args[0] * times + tag.length()
    return args
}

annotation retry(times: int, tag: string) {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(bump, [capture("times", times), capture("tag", tag)]))
  }
}

@retry("x", "y")
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
        let (result, _) = compile_source(src);
        let message = result
            .expect_err("a string application arg against `times: int` must reject loudly")
            .to_string();
        assert!(
            message.contains("int") || message.contains("type"),
            "the rejection names the type mismatch (contains-level; S5 owns attribution): {message}"
        );
    }

    // NON-VACUITY CONTROL for the scanner: a fn that GENUINELY reads a
    // top-level script binding loads it via `LoadModuleBinding` (the
    // opcode_defs.rs:951-documented load path), so the same scanner counts
    // > 0 here — proving the zero assertions above are falsifiable.
    #[test]
    fn module_binding_load_scanner_counts_a_genuine_module_read() {
        let src = "let base = 7\n\
                   fn reads_module() -> int { return base }\n\
                   reads_module()";
        let (value, compiler) = top_level_i64(src);
        assert_eq!(value, 7, "the module read executes");
        let reader = function_entry(&compiler, "reads_module");
        let count = module_binding_loads(&compiler, reader);
        assert!(
            count > 0,
            "control: a genuine top-level-binding read must be counted (got {count})"
        );
        // The S0 note names the legacy opcode itself: at HEAD the typed load
        // variants are emitter-dead, so the counted load IS `LoadModuleBinding`
        // (0x52) — the exact opcode the a4d wrapper measurement recorded.
        let end = reader.entry_point + reader.body_length;
        assert!(
            compiler.program.instructions[reader.entry_point..end]
                .iter()
                .any(|instr| instr.opcode == crate::bytecode::OpCode::LoadModuleBinding),
            "control: the S0-named opcode (LoadModuleBinding, 0x52) is the emitted load"
        );
    }

    // ── ADR-009 C3 #14 (slice 5, S5a): the [C0926] ambient-totality gate +
    // the a1–a6 disposition matrix + the [C0931] Dec-65 config-arg check.
    //
    // THE DISPOSITION TABLE (each row pinned below; verdicts per the S5
    // design — legit-input / declared-capture / [C0926] / [C0931] / G12):
    //
    // | row | shape                                        | verdict |
    // |-----|----------------------------------------------|---------|
    // | a1  | script top-level `let` read in template body | [C0926] |
    // | a2  | `pub let` in a mod block                     | unchanged pre-existing const-requirement rejection (control) |
    // | a2b | annotation's own module `pub const`          | [C0926] + the capture positive twin (the legitimate-intent case) |
    // | a3  | binding only in the TARGET's module          | [C0926] |
    // | a4  | nested-fn application                        | C3-G12 loud rejection (TypedConfig landed S4 — `reject_typed_config_annotations_on_nested_fn`; Legacy extension is S5b) |
    // | a4d | runtime module binding as config arg         | [C0931] |
    // | a5  | config-param-only body                       | LEGIT — the surviving path (executed twin) |
    // | a6  | defs const + same-spelled target binding     | [C0926] — the headline shadow disaster |
    //
    // Pre-gate hole measurements (S5a probes, throwaway, reverted; recorded
    // in c3-slice5-report.md): P1 (a1 shape) compiled + ran SILENTLY with
    // the ambient value (110) and one LoadModuleBinding in the specialized
    // handler; P2c (a6 sugar shape) compiled + ran SILENTLY with the
    // application-module shadow value (1040, not 160). `ctx` stays an
    // ordinary unresolved identifier (E4 family — no arm).
    mod s5a_ambient_totality {
        use super::*;

        /// The exact [C0926] sentence for one (origin, ident, resolved)
        /// triple — the pin-side mirror of the ONE producer
        /// (`pseudo_tuple::AmbientScopeCtx::ambient_rejection`).
        fn c0926_sentence(origin: &str, ident: &str, resolved: &str) -> String {
            format!(
                "[C0926] hook template body {origin} references `{ident}`, which resolves to \
                 the module-scope value binding `{resolved}`; a hook template's body reads only \
                 its exact inputs — its signature parameters and its declared captures (C3-G4); \
                 module- and invocation-scope values never enter a template ambiently, because \
                 the body is specialized into the application module where an unrelated binding \
                 spelled `{ident}` silently takes over the hook's behavior — declare it as an \
                 input instead: add a typed config parameter (or capture(\"{ident}\", ...)) and \
                 reference the capture parameter"
            )
        }

        /// Expect a rejection and return (message, location). The [C0926]
        /// SemanticError passes through the directive-processing wrap
        /// INTACT (the template_application_error provenance note satisfies
        /// the D1 preserve predicate), so the location is the
        /// `@application` anchor — never a handler-span RuntimeError.
        fn expect_semantic_error(src: &str) -> (String, Option<shape_ast::error::SourceLocation>) {
            let (result, _) = compile_source(src);
            match result.expect_err("fixture must be rejected") {
                shape_ast::error::ShapeError::SemanticError { message, location } => {
                    (message, location)
                }
                other => panic!("expected a SemanticError, got: {other}"),
            }
        }

        // a1 — SENTENCE-EXACT pin + span: the P1 silent-hole fixture now
        // rejects with the full [C0926] sentence anchored at the
        // `@application` line.
        #[test]
        fn a1_toplevel_let_in_template_body_rejects_with_the_exact_sentence() {
            let src = r#"
let ambient = 7

fn hook(x: int) -> int { return x + ambient }

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, location) = expect_semantic_error(src);
            assert!(
                message.contains(&c0926_sentence("fn `hook`", "ambient", "ambient")),
                "the full [C0926] sentence must appear byte-exact: {message}"
            );
            let location = location.expect("the rejection carries the @application span");
            let application_line = src
                .lines()
                .position(|line| line.starts_with("@hookann"))
                .expect("fixture has the application line")
                + 1;
            assert_eq!(
                location.line, application_line,
                "the rejection anchors at the @application site, not the template body"
            );
        }

        // a2 — CONTROL: `pub let` in a mod block keeps its pre-existing
        // unchanged rejection (module-level variables require const).
        #[test]
        fn a2_mod_level_let_control_keeps_the_preexisting_rejection() {
            let src = "mod m {\n    pub let x = 5\n}\nlet y = 1\ny";
            let (result, _) = compile_source(src);
            let err = result.expect_err("mod-level let is rejected today");
            assert!(
                err.to_string()
                    .contains("module-level variable declarations currently require `const`"),
                "the pre-existing const-requirement rejection is unchanged: {err}"
            );
            assert!(
                !err.to_string().contains("[C0926]"),
                "a2 is NOT a [C0926] case — the pre-existing rule owns it: {err}"
            );
        }

        // a2b — the LEGITIMATE-INTENT case: the annotation's OWN module
        // `pub const` read by its sugar hook body. Pre-gate this failed as a
        // bland `Undefined variable: 'secret'` (probe P2/P2b — mod consts
        // never resolved in fn bodies at all); the gate upgrades it to the
        // named [C0926] whose positive twin says HOW to do it right
        // (declare it as a capture / typed config parameter). The
        // resolution comes from the TEMPLATE-module trial (`defs::secret`).
        #[test]
        fn a2b_annotations_own_module_const_rejects_with_the_capture_twin() {
            let src = r#"
mod defs {
    pub const secret: int = 11

    annotation hookann(times: int) {
      targets: [function]
      before(args) {
        args[0] = args[0] + secret + times
        return args
      }
    }
}

@defs::hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains(&c0926_sentence(
                    "the `before` hook of annotation `hookann`",
                    "secret",
                    "defs::secret"
                )),
                "a2b: the sugar-origin [C0926] sentence with the defs-module resolution: \
                 {message}"
            );
        }

        // a3 — a binding that lives ONLY in the application module, read by
        // the annotation's (defs-module) sugar hook body.
        #[test]
        fn a3_target_module_binding_rejects_c0926() {
            let src = r#"
let sees = 9

mod defs {
    annotation hookann(times: int) {
      targets: [function]
      before(args) {
        args[0] = args[0] + sees + times
        return args
      }
    }
}

@defs::hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains(&c0926_sentence(
                    "the `before` hook of annotation `hookann`",
                    "sees",
                    "sees"
                )),
                "a3: the application-module binding is ambient for the defs-declared hook: \
                 {message}"
            );
        }

        // a6 — THE HEADLINE (the motivating disaster quoted in the
        // producer's doc-comment): the annotation's own `defs::secret = 11`
        // AND a same-spelled application-module `let secret = 99`. Pre-gate
        // (probe P2c, this exact fixture): compiled and ran SILENTLY with
        // the shadow value — 1040, not the annotation-intent 160. The
        // committed pin asserts the rejection names the APPLICATION-module
        // binding (`secret`) — the exact silent winner.
        #[test]
        fn a6_shadow_probe_rejects_naming_the_application_module_binding() {
            let src = r#"
mod defs {
    pub const secret: int = 11

    annotation hookann(times: int) {
      targets: [function]
      before(args) {
        args[0] = args[0] + secret + times
        return args
      }
    }
}

let secret = 99

@defs::hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains(&c0926_sentence(
                    "the `before` hook of annotation `hookann`",
                    "secret",
                    "secret"
                )),
                "a6: the rejection must name the application-module binding (the silent \
                 winner), not the defs const: {message}"
            );
        }

        // a5 — the SURVIVING LEGIT path, EXECUTED (config-param-only body):
        // value-distinguishing run + the weave-tier ambient belt (the
        // extended S3c scanner claim: post-gate, specialized handlers and
        // generated wrappers carry ZERO module-binding loads — the
        // genuine-module-read non-vacuity control lives above).
        #[test]
        fn a5_config_only_body_weaves_runs_and_stays_module_binding_free() {
            let src = r#"
annotation tagged(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

@tagged(3)
fn victim(a: int) -> int { return a + 1 }

victim(4)
"#;
            let (value, compiler) = top_level_i64(src);
            assert_eq!(value, 13, "4*3 = 12 → impl(12) = 13 (skip ⇒ 5; misread times shifts)");
            assert_eq!(compiler.hook_install_registry.len(), 1);
            for row in &compiler.hook_install_registry {
                let handler = &compiler.program.functions[usize::from(row.function_index)];
                assert_eq!(
                    module_binding_loads(&compiler, handler),
                    0,
                    "ambient belt: the specialized handler is module-binding-free"
                );
            }
            let wrapper = function_entry(&compiler, "victim");
            assert_eq!(
                module_binding_loads(&compiler, wrapper),
                0,
                "ambient belt: the generated wrapper is module-binding-free"
            );
        }

        // MUST-SCAN: f-string interpolation interiors (the boundary the
        // pseudo-tuple faces skip — pre-gate an ambient name there RESOLVED
        // and was silently honored).
        #[test]
        fn fstring_interpolation_interior_is_scanned() {
            let src = r#"
let ambient = 7

fn hook(x: int) -> int {
    let s = f"{ambient}"
    return x + 1
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`ambient`"),
                "an ambient name inside an f-string interpolation rejects: {message}"
            );
        }

        /// The exact F5-boundary sentence for one (origin, ident,
        /// args-spelling) triple — the pin-side mirror of the ONE producer
        /// (`pseudo_tuple::AmbientScopeCtx::fstring_template_name_rejection`,
        /// fix round 1, F1).
        fn fstring_template_name_sentence(origin: &str, ident: &str, args: &str) -> String {
            format!(
                "hook template body {origin} references the template name `{ident}` inside an \
                 f-string interpolation; interpolation interiors are raw text to the \
                 pseudo-tuple specialization (the named non-scanned boundary), so `{ident}` is \
                 never resolved there and would instead resolve ambiently in the application \
                 module — hoist the value to a local outside the f-string (for example \
                 `let v = {args}[0]`) and interpolate the local (`f\"{{v}}\"`)"
            )
        }

        // F1 (fix round 1) — the a6 class INSIDE an f-string interior: the
        // template's own pseudo-tuple spelling in an interpolation, with a
        // same-spelled application-module binding poised to win silently at
        // emission (the Validate/Rewrite faces never scan interiors; module
        // bindings resolve before fn tables). Rejects with the named
        // F5-boundary sentence, byte-exact. The annotation carries a TYPED
        // config param (fixture written under the deleted S4 classification
        // selector; post-collapse every def routes this surface).
        #[test]
        fn f1_fstring_interior_template_name_rejects_with_the_exact_sentence() {
            let src = r#"
let args = [7, 8]

annotation hookann(times: int) {
  targets: [function]
  before(args) {
    let s = f"{args[0]}"
    args[0] = args[0] + times
    return args
  }
}

@hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains(&fstring_template_name_sentence(
                    "the `before` hook of annotation `hookann`",
                    "args",
                    "args"
                )),
                "the full F5-boundary sentence must appear byte-exact: {message}"
            );
        }

        // F1 POSITIVE TWIN, EXECUTED: hoist the value to a local outside
        // the f-string — the woven program runs correctly even with the
        // same-spelled application-module binding present, and the
        // specialized handler stays module-binding-free (the ambient belt).
        #[test]
        fn f1_hoisted_local_twin_weaves_runs_and_stays_module_binding_free() {
            let src = r#"
let args = [7, 8]

annotation tagged(times: int) {
  targets: [function]
  before(args) {
    let v = args[0]
    let s = f"{v}"
    args[0] = args[0] + times
    return args
  }
}

@tagged(2)
fn victim(a: int) -> int { return a + 1 }

victim(4)
"#;
            let (value, compiler) = top_level_i64(src);
            assert_eq!(value, 7, "4+2 = 6 → impl(6) = 7 (skip ⇒ 5; ambient [7,8] shifts)");
            assert_eq!(compiler.hook_install_registry.len(), 1);
            for row in &compiler.hook_install_registry {
                let handler = &compiler.program.functions[usize::from(row.function_index)];
                assert_eq!(
                    module_binding_loads(&compiler, handler),
                    0,
                    "ambient belt: the specialized handler is module-binding-free"
                );
            }
        }

        // F2 (fix round 1) — a capture CLAUSE naming a module binding: the
        // entries are references into the OUTER environment, so `share bump`
        // on a template-body closure is the a6 hazard through the clause
        // (move-mode is C0906-gated downstream, but share-mode would lower
        // to the shared capture kind silently). The ambient face classifies
        // entry names BEFORE binding them into the closure frame.
        #[test]
        fn f2_capture_clause_module_binding_rejects_c0926() {
            let src = r#"
let bump = 5

mod defs {
    annotation hookann(times: int) {
      targets: [function]
      before(args) {
        let f = |y: int; share bump| y + 1
        args[0] = f(args[0])
        return args
      }
    }
}

@defs::hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains(&c0926_sentence(
                    "the `before` hook of annotation `hookann`",
                    "bump",
                    "bump"
                )),
                "f2: a capture-clause entry naming a module binding rejects [C0926]: {message}"
            );
        }

        // MUST-SCAN: assignment TARGETS (a store into a module binding is
        // as ambient as a read — StoreModuleBinding).
        #[test]
        fn assignment_target_module_binding_is_scanned() {
            let src = r#"
let mut counter = 0

fn hook(x: int) -> int {
    counter = counter + 1
    return x
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`counter`"),
                "a module-binding assignment target rejects: {message}"
            );
        }

        // MUST-SCAN: closure interiors.
        #[test]
        fn closure_interior_ambient_reference_is_scanned() {
            let src = r#"
let bump = 5

fn hook(x: int) -> int {
    let f = |y: int| y + bump
    return f(x)
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`bump`"),
                "an ambient reference inside a closure rejects: {message}"
            );
        }

        // MUST-SCAN: module-QUALIFIED value references (`defs::secret`
        // parses as a Unit-payload enum-constructor path).
        #[test]
        fn module_qualified_value_reference_is_scanned() {
            let src = r#"
mod defs {
    pub const secret: int = 11

    annotation hookann(times: int) {
      targets: [function]
      before(args) {
        args[0] = args[0] + defs::secret + times
        return args
      }
    }
}

@defs::hookann(1)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`defs::secret`"),
                "a module-qualified value reference rejects: {message}"
            );
        }

        // NEGATIVE CONTROL: template-local bindings are NOT ambient — a
        // local `let` of the same spelling as a module binding stays legit
        // and the woven program runs with the LOCAL value.
        #[test]
        fn template_local_binding_shadows_module_binding_and_runs() {
            let src = r#"
let bump = 7

fn hook(x: int) -> int {
    let bump = 2
    return x + bump
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (value, _) = top_level_i64(src);
            assert_eq!(value, 60, "the LOCAL bump (2) drives the hook: (4+2)*10; ambient 7 ⇒ 110");
        }

        // Sequential visibility: a use BEFORE the same-spelled local `let`
        // resolves ambiently — rejected.
        #[test]
        fn use_before_local_let_of_same_spelling_is_ambient() {
            let src = r#"
let bump = 7

fn hook(x: int) -> int {
    let y = bump
    let bump = 2
    return x + y + bump
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`bump`"),
                "a use before the local let resolves ambiently and rejects: {message}"
            );
        }

        // Disjoint-branch shadow: a local binder in one branch does NOT
        // mask an ambient use after the branch (the frame pops).
        #[test]
        fn disjoint_branch_local_does_not_mask_ambient_use() {
            let src = r#"
let bump = 7

fn hook(x: int) -> int {
    if x > 100 {
        let bump = 1
        let z = bump
    }
    return x + bump
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (message, _) = expect_semantic_error(src);
            assert!(
                message.contains("[C0926]") && message.contains("`bump`"),
                "the branch-local binder pops with its frame; the later use is ambient: \
                 {message}"
            );
        }

        // NEGATIVE CONTROL (the G4 boundary rule): fn-callees at module
        // scope are LEGIT (G3 — bodies call helper fns).
        #[test]
        fn module_fn_callees_stay_legit_and_run() {
            let src = r#"
fn helper(v: int) -> int { return v + 1 }

fn hook(x: int) -> int { return helper(x) }

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
            let (value, _) = top_level_i64(src);
            assert_eq!(value, 50, "helper(4) = 5 → impl(5) = 50 — module fn callees are code");
        }

        // ── Dec-65: the [C0931] config-arg pre-check + the
        // pinned-unconstructible hook-input shapes ──────────────────────────

        /// The exact [C0931] sentence — the pin-side mirror of the ONE
        /// producer (`reject_runtime_module_binding_config_args`).
        fn c0931_sentence(ident: &str, ann: &str) -> String {
            format!(
                "[C0931] config argument `{ident}` for `@{ann}` references a runtime module \
                 binding; annotation config is evaluated once at compile time (Dec 65 — \
                 runtime values never enter a comptime evaluation position) — pass a literal \
                 or a comptime const; a value that varies at runtime cannot configure a \
                 compile-time specialization"
            )
        }

        // a4d analog, TypedConfig class (probe P3a upgraded): pre-check
        // fires with the named sentence at the @application span — the
        // pre-gate text was the bland mini-VM `[C0001] Undefined variable:
        // chosen`.
        #[test]
        fn c0931_typed_config_runtime_binding_config_arg() {
            let src = r#"
let chosen = 5

annotation retry(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

@retry(chosen)
fn victim(a: int) -> int { return a }

victim(4)
"#;
            let (message, location) = expect_semantic_error(src);
            assert!(
                message.contains(&c0931_sentence("chosen", "retry")),
                "the full [C0931] sentence must appear byte-exact: {message}"
            );
            let location = location.expect("the rejection carries the @application span");
            let application_line = src
                .lines()
                .position(|line| line.starts_with("@retry"))
                .expect("fixture has the application line")
                + 1;
            assert_eq!(location.line, application_line);
        }

        // ADR-009 C3-S6 completion: `c0931_legacy_comptime_runtime_binding_
        // config_arg` RETIRED — its untyped `annotation amb(cfg)` fixture now
        // rejects at the DECLARATION (the collapsed untyped-config rejection,
        // surface_class.rs) before [C0931] can fire; the typed sibling above
        // carries the [C0931] class.

        // The invariant-7 CONST EXEMPTION: a top-level `const` config arg is
        // NOT [C0931] (a const is comptime-evaluable — never "a runtime
        // module binding"). MEASURED RESIDUAL (probe P6, disclosed): the
        // mini-VM has no const-injection route at HEAD, so the exempted
        // const still fails with the PRE-EXISTING loud `[C0001] Undefined
        // variable` — pinned here so the exemption's meaning (never
        // mis-fire) and the visibility gap (named follow-up) are both
        // locked. The twin that RUNS today is the literal config arg (a5).
        #[test]
        fn c0931_const_config_arg_is_exempt_and_keeps_the_preexisting_loud_error() {
            let src = r#"
const n: int = 3

annotation retry(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

@retry(n)
fn victim(a: int) -> int { return a }

victim(4)
"#;
            let (result, _) = compile_source(src);
            let err = result.expect_err("the const-visibility gap keeps this loud today");
            let text = err.to_string();
            assert!(
                !text.contains("[C0931]"),
                "a comptime const must NEVER be called a runtime module binding: {text}"
            );
            assert!(
                text.contains("Undefined variable: n"),
                "the pre-existing loud mini-VM failure is unchanged (probe P6): {text}"
            );
        }

        // Dec-65 (i) — PINNED-UNCONSTRUCTIBLE (E2-D9 precedent): a hook
        // INPUT name (`args`) referenced in HANDLER scope dies loud in the
        // mini-VM BEFORE any evaluation (probe P4, text locked here). No
        // dead product arm exists for this shape.
        #[test]
        fn dec65_hook_input_in_handler_scope_dies_loud_preevaluation() {
            let src = r#"
fn hook<Args>(args: Args, x: int) -> Args {
    args[0] = args[0] + x
    return args
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, [capture("x", args)]))
  }
}

@hookann()
fn victim(a: int) -> int { return a }

victim(4)
"#;
            let (result, _) = compile_source(src);
            let err = result.expect_err("hook inputs are unresolvable in handler scope");
            assert!(
                err.to_string().contains("Undefined variable: 'args'"),
                "the loud pre-evaluation failure is the locked behavior: {err}"
            );
        }

        // Dec-65 (ii) — PINNED-UNCONSTRUCTIBLE: a `comptime` block inside a
        // template body reading a CONCRETE sig param dies loud in the fresh
        // mini-VM (probe P5 — `execute_comptime_with_context` receives
        // helpers only, no fn params; text locked here).
        #[test]
        fn dec65_comptime_block_reading_concrete_sig_param_dies_loud() {
            let src = r#"
fn hook(x: int) -> int {
    comptime {
        let y = x
    }
    return x + 1
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(hook, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a }

victim(4)
"#;
            let (result, _) = compile_source(src);
            let err = result.expect_err("sig params are invisible to a comptime block");
            assert!(
                err.to_string().contains("Undefined variable: 'x'"),
                "the loud pre-evaluation failure is the locked behavior: {err}"
            );
        }

        // Dec-65 (ii), the PSEUDO-TUPLE spelling — the S5a walker arm: an
        // `args` read inside a comptime block names the real boundary
        // instead of leaking a minted `__c3_arg_{i}` unresolved error.
        #[test]
        fn dec65_comptime_block_reading_pseudo_tuple_names_the_boundary() {
            let src = r#"
fn tmpl<Args>(args: Args) -> Args {
    comptime {
        let y = args.length
    }
    return args
}

annotation hookann() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a }

victim(4)
"#;
            let (result, _) = compile_source(src);
            let err = result.expect_err("the pseudo-tuple is not readable inside comptime");
            assert!(
                err.to_string().contains(
                    "a `comptime` block inside a template body cannot read `args`"
                ),
                "the named Dec-65 walker sentence fires: {err}"
            );
            assert!(
                !err.to_string().contains("__c3_"),
                "no minted reserved name leaks into the diagnostic: {err}"
            );
        }
    }
}

