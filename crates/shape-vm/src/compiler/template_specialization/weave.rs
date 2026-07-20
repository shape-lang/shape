//! ADR-009 C3 #14 (slice 2, S2c) — the typed-AST weave: materialize the
//! staged hook installs for one target as a GENERATED ORDINARY TYPED AST
//! wrapper + a journaled hygienic impl shadow.
//!
//! # The C3-G6 SMALL shape (slice-0 §2 — binding)
//!
//! The wrapper is an ordinary typed AST `FunctionDef` compiled through the
//! ORDINARY pipeline, so bytecode AND MIR derive from the same wrapped
//! definition — full native, no `mir_data` suppression. Un-suppressing
//! `mir_data` on the LEGACY raw-bytecode weave is MEASURED-FORBIDDEN
//! (slice-0 §2.3: silent VM≠JIT divergence, hooks silently skipped); the
//! legacy weave (`compile_wrapped_function` / `compile_chained_annotations` /
//! the `functions.rs:993-1006` suppression) stays BYTE-UNCHANGED beside this
//! module. A new-path target can never reach the legacy classification: the
//! apply seam's mixed-legacy rejection guarantees no annotation on the
//! target carries a legacy `before`/`after` handler, so
//! `find_compiled_annotations` returns empty and the wrapper compiles as an
//! ordinary fn with the `functions.rs:1132` mir-attach tail. This module
//! asserts that invariant defensively rather than routing around the legacy
//! classifier.
//!
//! # The weave shape
//!
//! Materialized ONCE per target, after the LAST handler + body directives
//! (so it wraps the FINAL — possibly `replace body`-edited — definition):
//!
//! 1. The final target body moves under an unspellable hygienic shadow name
//!    ([`HygienicRole::TemplateWeaveImplBody`], the C3 successor of the
//!    legacy `AnnotationHookImplBody` role), reserved through
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
//!   the weave, a legacy classification on a new-path target) are
//!   internal-error-shaped — the apply seam's rejections make them
//!   unreachable from user code.

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
    /// name) — the `original_body_shadow_name` /
    /// `annotation_hook_impl_name` precedent.
    fn template_weave_impl_name(&self, func_name: &str) -> String {
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

        // Defensive invariants (module docs): unreachable from user code —
        // the apply seam's G8 / mixed-legacy rejections fire first.
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
        if !self.find_compiled_annotations(func_def).is_empty() {
            return Err(internal(format!(
                "internal error: hook-template weave target `{}` classifies as a LEGACY \
                 runtime-hook wrapper (`find_compiled_annotations` non-empty) — the woven \
                 wrapper's mir_data would be suppressed (functions.rs:1046) and the JIT would \
                 silently skip the hooks; the apply seam's mixed-legacy rejection fires \
                 before any install stages",
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
        // ORDINARY fn — mir_data attached (the legacy weave's suppression
        // would leave it None) — and the hygienic shadow is registered with
        // its OWN mir_data (the slice-0 "(ii)" plumbing).
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
    // The S0 §4 a4d/a4e-noted JIT poison: the LEGACY config path reads its
    // config from a module binding at every invocation, planting
    // `LoadModuleBinding` (0x52) in the generated wrapper ("generated
    // wrapper contains `LoadModuleBinding` → W39 whole-program deopt"; the
    // legacy machinery stays byte-unchanged beside the new path until S6).
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
}

