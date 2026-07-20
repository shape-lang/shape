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
//!    `after` chain threading the typed result, also in application order.
//!    Capture values append as TYPED LITERAL trailing args per
//!    [`CaptureBindingPlan::CallSiteArgs`] at every handler call site.
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
//! - Internal invariants (a Before install without a carrier, an After
//!   install on a void target, a generic def reaching the weave, a legacy
//!   classification on a new-path target) are internal-error-shaped — the
//!   apply seam's rejections make them unreachable from user code.

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
use super::const_lift::CaptureBindingPlan;
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
        let capture_args = |install: &StagedHookInstall| -> Vec<Expr> {
            let CaptureBindingPlan::CallSiteArgs(args) = &install.capture_plan;
            args.clone()
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
            let mut args = current_args.clone();
            args.extend(capture_args(install));
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
                         before handler",
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
            // The `after` chain, in application order, threading the typed
            // result.
            for install in afters {
                let symbol = handler_symbol(self, install)?;
                let mut args = vec![result_expr];
                args.extend(capture_args(install));
                let local = fresh_local(&taken);
                stmts.push(decl(&local, return_annotation.clone(), call(&symbol, args)));
                result_expr = ident(&local);
            }
            stmts.push(Statement::Return(Some(result_expr), span));
        } else {
            if let Some(install) = afters.first() {
                return Err(internal(format!(
                    "internal error: staged `after` install (via @{}) on the void target \
                     `{}`; specialize_template rejects after-templates on targets without a \
                     typed result",
                    install.annotation_name, func_def.name
                )));
            }
            stmts.push(Statement::Expression(impl_expr, span));
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

    // ── captures: distinct values across ONE shared specialized handler ────

    // Two annotations install the SAME polymorphic template at the SAME Sig
    // with DIFFERENT capture values: the specialized handler is SHARED (the
    // cache key is value-generic — invariants resolution 5) while each
    // weave delivers its own capture literal at the call site.
    #[test]
    fn captures_deliver_distinct_values_across_a_shared_handler() {
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
        // victim_a: 10*3 = 30 → 31; victim_b: 10*5 = 50 → 51.
        assert_eq!(value, 31051, "each weave delivers its own capture literal");
        assert_eq!(compiler.hook_install_registry.len(), 2);
        let (row_a, row_b) = (
            &compiler.hook_install_registry[0],
            &compiler.hook_install_registry[1],
        );
        assert_eq!(
            row_a.function_index, row_b.function_index,
            "one Sig ⇒ ONE shared value-generic specialized handler (the cache-share pin)"
        );
        assert_ne!(
            row_a.captures, row_b.captures,
            "the installs differ ONLY in their capture literals"
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

    // Two AFTER installs stack in application order too: impl(1) = 10 →
    // +10 = 20 → *2 = 40 (reversed ⇒ 30).
    #[test]
    fn stacked_after_installs_thread_the_result_in_application_order() {
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
        assert_eq!(value, 40, "after chain in application order (reversed ⇒ 30)");
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
}

