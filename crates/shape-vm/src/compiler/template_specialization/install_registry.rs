//! ADR-009 C3 #14 (slice 2, S2b) — the hook-template INSTALL registry + the
//! pass-2 apply seam.
//!
//! # The apply seam (`BytecodeCompiler::apply_install_hook_template`)
//!
//! The `ComptimeDirective::InstallHookTemplate` consumer for the authoritative
//! pass-2 function-target phase (`process_comptime_directives_for_function`).
//! Per install, IN ORDER: resolve the handle from the still-intact per-run
//! store (`comptime_hook_template_at` — safe post-execute per the store's
//! lifecycle doc: execute-populated stores clear at the PRE-execute point, so
//! the store survives until the next handler run on this thread) → the C3-G8
//! generic-target named rejection → the mixed-legacy rejection (one weave
//! owner per target until S6 deletes the legacy machinery) →
//! [`super::SpecializationTarget`] glue → `specialize_template` →
//! `bind_captures_for_install` → stage the install on the caller's per-target
//! accumulator + write one journaled registry row.
//!
//! # Transaction composition (E1-D6b — why the precondition holds)
//!
//! `specialize_template`'s first statement REQUIRES the already-open C2
//! `InstallTransaction` (`template_specialization/mod.rs:221-229`). At every
//! production reach of this seam the journal is open BY CONSTRUCTION:
//! `compile_in_place` (`compiler_impl_reference_model.rs:1985-1996`) runs
//! `begin_checked_body_install()` at `:1986` BEFORE `compile_in_place_inner`
//! at `:1987`, and every annotation handler (and therefore every directive
//! consumer) executes inside that inner driver — never a second transaction.
//!
//! # The registry (the S8 hover/query substrate)
//!
//! `BytecodeCompiler::hook_install_registry` is a compiler-owned table (the
//! C1 slice-4 `generated_symbol_query` precedent: tooling reads compiler
//! query state, never a text scan). One row per applied install: annotation
//! name, target name, hook kind, the template's declared-Sig rendering, the
//! specialized symbol + function index, the capture names with their
//! `LiftedConst` renderings, and the `@application` span. Rows are written at
//! apply time and JOURNALED through the open `InstallTransaction`
//! (displaced-entry undo per the `checked_body/journal.rs` precedent —
//! `journal_record_hook_install_row` records the pre-write length; rollback
//! truncates back), so a failing compile leaves NO row: the registry rows are
//! transaction-scoped exactly like every other install publication.
//!
//! # Staging, not weaving (S2b stops here)
//!
//! [`StagedHookInstall`]s ACCUMULATE per target across its annotations on a
//! parameter-threaded accumulator (the `pending_original_body_shadow`
//! pattern — parameter, never ambient state). The weave materializes ONCE
//! after the last handler, wrapping the final (possibly replace-body-edited)
//! def in application order — that materialization is S2c; this stage stops
//! at staged installs + registry rows.

use shape_ast::ast::{FunctionDef, Span};
use shape_ast::error::{Result, ShapeError};

use super::const_lift::{self, CaptureBindingPlan};
use super::{SpecializedHandler, render_template_declared_signature};
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::ExpansionSite;
use crate::compiler::comptime_builtins::{BoundTemplate, comptime_hook_template_at};
use crate::compiler::comptime_fragments::checked_template::TemplateHookKind;
use crate::compiler::comptime_target::type_annotation_to_string;

/// One applied hook-template install, staged for the S2c weave. Accumulated
/// per target across its annotations in APPLICATION ORDER (the order the
/// weave wraps in), threaded as a parameter like
/// `pending_original_body_shadow` — never ambient compiler state.
#[derive(Debug)]
pub(in crate::compiler) struct StagedHookInstall {
    /// The specialized handler (function index + mutation carrier).
    pub(in crate::compiler) handler: SpecializedHandler,
    /// How the weave delivers the template's capture values at the handler
    /// call sites (S2: typed literals in trailing-parameter order).
    pub(in crate::compiler) capture_plan: CaptureBindingPlan,
    /// Before or After — which side of the target the weave attaches to.
    pub(in crate::compiler) hook_kind: TemplateHookKind,
    /// The installing annotation's name (origin, parameter-threaded).
    pub(in crate::compiler) annotation_name: String,
    /// The `@application` anchor (origin, parameter-threaded).
    pub(in crate::compiler) application_span: Span,
    /// The handler application's `ExpansionSite` (origin, parameter-threaded)
    /// — the S2c weave derives the shadow reservation's `GeneratedOrigin` +
    /// anchors from the FIRST staged install's site (the `CheckedReplaceBody`
    /// shadow-construction precedent).
    pub(in crate::compiler) site: ExpansionSite,
}

/// One row of the hook-install registry (module docs above): the S8
/// hover/query substrate and the sugar-matrix hover row. Renderings are
/// captured at apply time so the query surface never re-derives them.
#[derive(Debug, Clone)]
pub(in crate::compiler) struct HookInstallRecord {
    /// The installing annotation's name.
    pub(in crate::compiler) annotation_name: String,
    /// The target function's user-spelled name.
    pub(in crate::compiler) target_name: String,
    /// Before or After.
    pub(in crate::compiler) hook_kind: TemplateHookKind,
    /// The template's declared-Sig rendering
    /// (`render_template_declared_signature`, prefixed by the body fn name).
    pub(in crate::compiler) template_sig: String,
    /// The specialized handler's registered symbol (the definition-compiled
    /// body fn for concrete templates; the suffixed unspellable
    /// specialization symbol for polymorphic ones).
    pub(in crate::compiler) specialized_symbol: String,
    /// The specialized handler's function index.
    pub(in crate::compiler) function_index: u16,
    /// Capture names with their `LiftedConst` renderings, in the body fn's
    /// trailing-parameter (delivery) order.
    pub(in crate::compiler) captures: Vec<(String, String)>,
    /// The `@application` span the install anchors at.
    pub(in crate::compiler) application_span: Span,
}

impl BytecodeCompiler {
    /// The pass-2 apply seam for one `InstallHookTemplate` directive (module
    /// docs above for the ordered sequence + the transaction-composition
    /// chain). `annotation_name` + `site` are the parameter-threaded origin;
    /// `staged` is the caller's per-target accumulator.
    pub(in crate::compiler) fn apply_install_hook_template(
        &mut self,
        template_index: usize,
        annotation_name: &str,
        func_def: &FunctionDef,
        site: &ExpansionSite,
        staged: &mut Vec<StagedHookInstall>,
    ) -> Result<()> {
        let application_span = site.application_span();

        // 1. Resolve the handle against the still-intact per-run store. A
        //    handle can only be minted by `before_hook`/`after_hook` with the
        //    just-pushed index, so a miss here is a broken lifecycle
        //    invariant — internal-error-shaped, never a user rejection.
        let Some(bound) = comptime_hook_template_at(template_index) else {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "internal error: install directive template index {template_index} is not \
                     live in this comptime execution (the hook-template store is per-run and \
                     must survive until the directive is consumed — see the \
                     COMPTIME_HOOK_TEMPLATES lifecycle doc)"
                ),
                location: Some(self.span_to_source_location(application_span)),
            });
        };

        // 2. C3-G8: installing on a GENERIC target is a named rejection at
        //    the `@application` site — a deliberate capability withdrawal
        //    (C3-G11), re-armed when #59 (monomorphization-origin re-arm)
        //    lands. The defections.md withdrawal entry lands with S5's
        //    rejection matrix (C3-G11 obligation).
        //
        //    MEASURED reach (disclosed): a generic def's pass-2 body compile
        //    is SKIPPED (`functions.rs` compile_function_with_generated_origin
        //    — "Skip compiling bodies of generic extend methods"), so the
        //    PRIMARY G8 firing site is the analysis pre-pass consumer
        //    (`apply_signature_directives_to_analysis_function`), the only
        //    consumer that observes the generic def's real `type_params`.
        //    This seam keeps two defensive twins of the same rejection:
        //    (a) the direct `type_params` check, should a generic def ever
        //    reach pass-2; (b) the specialization-origin check below — a
        //    monomorphized specialization carries the ORIGINAL's annotations
        //    with `type_params` cleared and a `{original}::{mono_key}` name
        //    (substitution.rs::substitute_function_def), so installing onto
        //    it IS installing onto the generic target per specialization,
        //    exactly what C3-G8 withdraws until #59.
        if func_def
            .type_params
            .as_ref()
            .is_some_and(|params| !params.is_empty())
        {
            return Err(ShapeError::SemanticError {
                message: generic_target_install_rejection_message(
                    bound.template.body_fn(),
                    annotation_name,
                    func_def,
                ),
                location: Some(self.span_to_source_location(application_span)),
            });
        }
        if let Some(original) = self.generic_origin_of_specialized_name(&func_def.name) {
            let original = original.clone();
            return Err(ShapeError::SemanticError {
                message: generic_target_install_rejection_message(
                    bound.template.body_fn(),
                    annotation_name,
                    &original,
                ),
                location: Some(self.span_to_source_location(application_span)),
            });
        }

        // 3. Mixed-legacy rejection: ONE weave owner per target until the S6
        //    capstone deletes the legacy machinery (C3-G7). Any annotation on
        //    this target carrying a legacy runtime `before`/`after` handler
        //    (compiled id or per-target template) would weave the same target
        //    through `compile_wrapped_function`/`compile_chained_annotations`.
        for ann in &func_def.annotations {
            let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else {
                continue;
            };
            let engages_legacy_weave = compiled.before_handler.is_some()
                || compiled.after_handler.is_some()
                || compiled.before_handler_template.is_some()
                || compiled.after_handler_template.is_some();
            if engages_legacy_weave {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot install hook template `{}` (via @{annotation_name}) on `{}`: \
                         annotation `@{}` on the same target engages the legacy before/after \
                         runtime-hook weave, and a target has exactly one weave owner until \
                         the legacy machinery is deleted (C3-G7 / S6); move all of `{}`'s \
                         hooks onto the typed hook-template surface",
                        bound.template.body_fn(),
                        func_def.name,
                        ann.name,
                        func_def.name,
                    ),
                    location: Some(self.span_to_source_location(application_span)),
                });
            }
        }

        // 4./5. Target glue + specialization through the ONE open transaction
        //    (module docs: the journal is open by construction — E1-D6b). The
        //    descriptor slot is `None`: the structural comparison path is
        //    complete; the frozen-identity fast path stays available to
        //    callers that already hold a `CallableDescriptor` (identity-only,
        //    slice-0 §7.4).
        let target = self.specialization_target_from_def(func_def, None, application_span)?;
        let handler = self.specialize_template(&bound.template, &target)?;

        // 6. Capture delivery plan (S2 scalars as typed call-site literals;
        //    S3 replaces the domain at the const_lift boundary).
        let capture_plan = const_lift::bind_captures_for_install(&bound, &target)?;

        // 7. One journaled registry row (displaced-entry undo: the journal
        //    records the pre-write length; rollback truncates back, so a
        //    failing compile leaves NO row).
        self.journal_record_hook_install_row();
        let specialized_symbol = self
            .program
            .functions
            .get(handler.function_index() as usize)
            .map(|function| function.name.clone())
            .unwrap_or_else(|| bound.template.body_fn().to_string());
        self.hook_install_registry.push(HookInstallRecord {
            annotation_name: annotation_name.to_string(),
            target_name: func_def.name.clone(),
            hook_kind: bound.template.hook_kind(),
            template_sig: format!(
                "{} {}",
                bound.template.body_fn(),
                render_template_declared_signature(&bound.template)
            ),
            specialized_symbol,
            function_index: handler.function_index(),
            captures: rendered_captures(&bound),
            application_span,
        });

        // 8. Stage for the S2c weave (application order = accumulator order).
        staged.push(StagedHookInstall {
            handler,
            capture_plan,
            hook_kind: bound.template.hook_kind(),
            annotation_name: annotation_name.to_string(),
            application_span,
            site: site.clone(),
        });
        Ok(())
    }

    /// The C3-G8 specialization-origin check: a monomorphized specialization
    /// is renamed `{original}::{mono_key}[::{semantic_suffix}...]`
    /// (`substitution.rs::substitute_function_def` + the semantic rename
    /// seam), so walk the `::` boundaries from the LONGEST prefix down and
    /// return the first registered def that is EXPLICITLY generic — that def
    /// is the generic target the install would silently attach to per
    /// specialization. A registered non-generic prefix (a genuinely
    /// module-qualified concrete fn) keeps walking and ultimately returns
    /// `None` — concrete fns never take the specialization rename.
    fn generic_origin_of_specialized_name(&self, specialized_name: &str) -> Option<&FunctionDef> {
        let mut end = specialized_name.len();
        while let Some(pos) = specialized_name[..end].rfind("::") {
            let prefix = &specialized_name[..pos];
            if let Some(original) = self.function_defs.get(prefix) {
                if original
                    .type_params
                    .as_ref()
                    .is_some_and(|params| !params.is_empty())
                {
                    return Some(original);
                }
            }
            end = pos;
        }
        None
    }

    /// Discard the registry rows staged for `target_name` when a
    /// `RemoveTarget` directive removes the function mid-handler-chain: a
    /// removed target installs no weave, so its rows would misreport an
    /// install that never materializes. The journaled length-undo stays
    /// correct (rollback truncation to a shorter prior length is unaffected
    /// by rows removed here; on commit the retained rows are exactly the
    /// live installs).
    pub(in crate::compiler) fn discard_hook_install_rows_for_target(&mut self, target_name: &str) {
        self.hook_install_registry
            .retain(|row| row.target_name != target_name);
    }
}

/// Render the capture bindings for the registry row: `(name, rendering)` in
/// the body fn's trailing-parameter (delivery) order — the same order
/// `bind_captures_for_install` delivers in.
fn rendered_captures(bound: &BoundTemplate) -> Vec<(String, String)> {
    bound
        .template
        .capture_params()
        .iter()
        .filter_map(|(param_name, _)| {
            bound
                .capture_values
                .iter()
                .find(|(name, _)| name == param_name)
                .map(|(name, lifted)| (name.clone(), lifted.render()))
        })
        .collect()
}

/// The ONE C3-G8 sentence producer — shared by the pre-pass firing site
/// (`apply_signature_directives_to_analysis_function`, which observes the
/// generic def's real `type_params`) and the seam's two defensive twins, so
/// the ruled wording (target + generic signature + the #59 re-arm cite + the
/// concrete-target positive twin) never forks.
pub(in crate::compiler) fn generic_target_install_rejection_message(
    template_body_fn: &str,
    annotation_name: &str,
    func_def: &FunctionDef,
) -> String {
    format!(
        "cannot install hook template `{template_body_fn}` (via @{annotation_name}) on `{}`: \
         the target is generic — `{}` — and hook-template installs on generic targets are \
         withdrawn until #59 (the monomorphization-origin re-arm) lands (C3-G8); \
         signature-polymorphic templates stay definable and usable on every concrete target — \
         apply @{annotation_name} to a concrete function",
        func_def.name,
        render_generic_target_signature(func_def),
    )
}

/// Render a generic target's signature for the C3-G8 rejection —
/// `fn name<T, U>(a: T, b: int) -> T`, from the AST side (user spellings,
/// never a mangled mono-key — the S0 g3 rule).
fn render_generic_target_signature(func_def: &FunctionDef) -> String {
    let type_params = func_def
        .type_params
        .as_ref()
        .map(|params| {
            params
                .iter()
                .map(|param| param.name().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let params = func_def
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("<pattern>");
            match &param.type_annotation {
                Some(annotation) => format!("{name}: {}", type_annotation_to_string(annotation)),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match &func_def.return_type {
        Some(annotation) => format!(
            "fn {}<{type_params}>({params}) -> {}",
            func_def.name,
            type_annotation_to_string(annotation)
        ),
        None => format!("fn {}<{type_params}>({params})", func_def.name),
    }
}

// ADR-009 C3 #14 (slice 2, S2b): end-to-end apply-seam pins through the
// PUBLIC comptime API over the full compile path (parse → pre-pass →
// analyzer → pass-2 handler → `install` directive → this module's apply
// seam), asserting COMPILER STATE (the journaled registry) and EXECUTING
// specialized handlers (the S1c stage-4 pattern — compile-proof alone is
// banned per the S0 §2 named uncertainty).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, GeneratorRef,
        TargetIdentity,
    };
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{KindedSlot, NativeKind};

    /// Whole-program fixture (the S2a `hook_template_builtin_tests` shape):
    /// module-scope body fns + one installing annotation + caller-supplied
    /// annotated targets (and optional top-level calls).
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

    /// Full production compile; returns BOTH the result and the compiler so
    /// pins can assert the post-compile (or post-rollback) registry state.
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

    fn expect_compile_reject(src: &str, needles: &[&str]) -> crate::compiler::BytecodeCompiler {
        let (result, compiler) = compile_source(src);
        let text = result.expect_err("fixture must reject").to_string();
        for needle in needles {
            assert!(
                text.contains(needle),
                "expected rejection containing {needle:?}, got: {text}"
            );
        }
        compiler
    }

    // IDENTITY PROOF (the S1c pattern) + the pre-pass single-apply pin:
    // a concrete install resolves the DEFINITION-COMPILED body fn's index,
    // and the pre-pass + pass-2 double handler run yields EXACTLY ONE
    // registry row (the pre-pass consumer arm is a documented no-op).
    #[test]
    fn concrete_install_records_the_definition_compiled_body_index_once() {
        let compiler = compiled_ok(&hook_source(
            "fn my_before(x: int) -> int { return x + 1 }",
            "install(before_hook(my_before, []))",
            "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
        ));
        assert_eq!(
            compiler.hook_install_registry.len(),
            1,
            "exactly one row across the pre-pass + pass-2 double handler run"
        );
        let row = &compiler.hook_install_registry[0];
        assert_eq!(row.annotation_name, "hookann");
        assert_eq!(row.target_name, "victim");
        assert_eq!(row.hook_kind, TemplateHookKind::Before);
        let body_index = compiler
            .find_function("my_before")
            .expect("the template body fn is registered");
        assert_eq!(
            row.function_index as usize, body_index,
            "a concrete install resolves the definition-compiled body index"
        );
        assert_eq!(row.specialized_symbol, "my_before");
        assert!(row.captures.is_empty());
        assert!(
            row.template_sig.contains("my_before") && row.template_sig.contains("(int) -> int"),
            "the row renders the template's declared Sig: {}",
            row.template_sig
        );
    }

    // EXECUTION PROOF (the S1c stage-4 pattern): a polymorphic install with
    // a scalar capture specializes through the monomorphization ride with
    // the CAPTURE TAIL PRESERVED — the specialized handler executes with
    // [target slots..., capture value], mutates slot 0 via the capture and
    // the `args.length` constant, and the capture literal is recorded on
    // the registry row.
    #[test]
    fn polymorphic_install_with_capture_executes_and_records_the_literal() {
        let compiler = compiled_ok(&hook_source(
            "fn tmpl<Args>(args: Args, factor: int) -> Args {\n\
             \x20   args[0] = args[0] * factor + args.length\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, [capture(\"factor\", 3)]))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        assert_eq!(compiler.hook_install_registry.len(), 1);
        let row = &compiler.hook_install_registry[0];
        assert_eq!(row.hook_kind, TemplateHookKind::Before);
        assert_eq!(
            row.captures,
            vec![("factor".to_string(), "3".to_string())],
            "the capture literal is recorded in delivery order"
        );
        assert!(
            row.specialized_symbol.contains("tmpl"),
            "the polymorphic install records the suffixed specialization symbol: {}",
            row.specialized_symbol
        );
        assert_ne!(
            row.specialized_symbol, "tmpl",
            "a polymorphic handler is a specialization, never the generic def itself"
        );

        // Direct-call execution proof: [a=5, b=4.5, factor=3] →
        // a0 = 5*3 + args.length(=2) = 17, a1 = 4.5 untouched.
        //
        // S2c UPDATE (execute by NAME, not by compiler-side index): the
        // row's `function_index` is a COMPILER-side program index, and
        // `VirtualMachine::load_program` links/relocates the shipped
        // program — its function ids need not match. At S2b the two
        // orderings happened to coincide; the S2c weave's extra generated
        // functions (hygienic shadow + woven wrapper) shifted the linked
        // order, so executing the raw index silently ran the WOVEN TARGET
        // instead of the handler. The row's `specialized_symbol` names the
        // handler unambiguously in both tables.
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        let result = vm
            .execute_function_by_name(
                &row.specialized_symbol,
                vec![
                    KindedSlot::from_int(5),
                    KindedSlot::from_number(4.5),
                    KindedSlot::from_int(3),
                ],
                None,
            )
            .expect("the specialized handler executes");
        let storage = result
            .as_typed_object_storage()
            .expect("the 2-ary mutation aggregate is the inline-schema TypedObject");
        assert_eq!(storage.field_kinds[0], NativeKind::Int64);
        assert_eq!(storage.slots()[0].raw() as i64, 17, "slot 0 mutated via capture + length");
        assert_eq!(storage.field_kinds[1], NativeKind::Float64);
        assert_eq!(f64::from_bits(storage.slots()[1].raw()), 4.5, "slot 1 flows through");
    }

    // THE VERIFY-1 REFUTER SHAPE THROUGH THE API: two targets whose FLAT
    // tuple renderings collide (the S1 injectivity fix's exact pair) must
    // specialize SEPARATELY when installed through the public builtins.
    #[test]
    fn colliding_render_sigs_installed_through_the_api_stay_separate() {
        let compiler = compiled_ok(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\x20   return args\n}",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn target_a(x: [int, int, int], y: number) -> int { return 1 }\n\n\
             @hookann()\nfn target_b(x: [int, int], y: int, z: number) -> int { return y }",
        ));
        assert_eq!(
            compiler.hook_install_registry.len(),
            2,
            "one row per annotated target"
        );
        let (first, second) = (
            &compiler.hook_install_registry[0],
            &compiler.hook_install_registry[1],
        );
        assert_eq!(first.target_name, "target_a");
        assert_eq!(second.target_name, "target_b");
        assert_ne!(
            first.function_index, second.function_index,
            "distinct Sigs must never share one checked specialization (verify-1)"
        );
    }

    // C3-G8: install on a GENERIC target is a named rejection — target name
    // + generic signature + the #59 re-arm cite + the concrete-target
    // positive twin — and no row lands. MEASURED REACH (disclosed): in a
    // single-module unit both pre-passes run BEFORE function registration,
    // so the body-fn rewrite defers every install handler to pass-2, and
    // pass-2 skips a generic def's body compile — the generic target's
    // handler therefore runs ONLY on the monomorphized specialization at a
    // CALL SITE (annotations survive substitution), where the seam's
    // specialization-origin twin fires naming the ORIGINAL generic
    // signature. (On graph-compiled units with an imported annotation +
    // body fn the pre-pass consumer arm fires the same sentence earlier;
    // an UNCALLED generic target in a single-module unit stays a silent
    // no-op — surfaced in the stage report for supervisor disposition.)
    #[test]
    fn generic_target_install_rejects_with_the_g8_sentence() {
        let compiler = expect_compile_reject(
            &hook_source(
                "fn my_before(x: int) -> int { return x + 1 }",
                "install(before_hook(my_before, []))",
                "@hookann()\nfn victim<T>(x: T) -> T { return x }\n\nvictim(1)",
            ),
            &[
                "cannot install hook template `my_before` (via @hookann) on `victim`",
                "fn victim<T>(x: T) -> T",
                "withdrawn until #59 (the monomorphization-origin re-arm)",
                "apply @hookann to a concrete function",
            ],
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // Non-function target: the pass-2 type-target consumer names the
    // rejection with its positive twin.
    #[test]
    fn type_target_install_rejects_with_the_function_twin() {
        expect_compile_reject(
            &format!(
                r#"
fn my_before(x: int) -> int {{ return x + 1 }}

annotation hookann() {{
  targets: [type]
  comptime post(target, ctx) {{
    install(before_hook(my_before, []))
  }}
}}

@hookann()
type Widget {{
  w: int
}}
"#
            ),
            &[
                "`install` directives are only valid when compiling function targets",
                "apply the installing annotation to a function",
            ],
        );
    }

    // Mixed legacy + new weave: ONE weave owner per target until S6.
    #[test]
    fn mixed_legacy_weave_target_rejects_the_install() {
        expect_compile_reject(
            &format!(
                r#"
fn my_before(x: int) -> int {{ return x + 1 }}

annotation legacy_hook() {{
  before(args, ctx) {{
    args
  }}
}}

annotation hookann() {{
  targets: [function]
  comptime post(target, ctx) {{
    install(before_hook(my_before, []))
  }}
}}

@legacy_hook()
@hookann()
fn victim(a: int) -> int {{ return a }}

victim(1)
"#
            ),
            &[
                "engages the legacy before/after runtime-hook weave",
                "exactly one weave owner",
                "typed hook-template surface",
            ],
        );
    }

    // The install builtin's own misuse rejection: a non-handle argument is
    // named with the producer twin.
    #[test]
    fn non_handle_install_argument_rejects_naming_the_producers() {
        expect_compile_reject(
            &hook_source(
                "fn my_before(x: int) -> int { return x + 1 }",
                "install(5)",
                "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
            ),
            &[
                "install expects a __CheckedTemplate handle",
                "before_hook(body_fn, captures) or after_hook(body_fn, captures)",
            ],
        );
    }

    // ROLLBACK PROBE (extends the S1c pin over the REGISTRY): the FIRST
    // target's install applies (row + cache entry), the SECOND target's
    // install fails at pass-2 (zero-param before), the whole compile rolls
    // back — NO registry row survives and the monomorphization cache is
    // evicted (checked_body/mod.rs fold-in). Non-vacuity: the error names
    // the SECOND target, proving the first had already applied in program
    // order before the rollback.
    #[test]
    fn failing_install_rolls_back_registry_rows_and_the_cache() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\x20   return args\n}",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\n\
             @hookann()\nfn zero() -> int { return 7 }",
        ));
        let text = result
            .expect_err("the zero-param before target must fail the compile")
            .to_string();
        assert!(
            text.contains("zero") && text.contains("declares no parameters"),
            "the failure comes from the SECOND target's install (non-vacuity): {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "rollback removes the first target's already-written registry row"
        );
        assert_eq!(
            compiler.monomorphization_cache.legacy_len(),
            0,
            "rollback evicts the first install's at/above-watermark cache entry"
        );
    }

    // Stale/out-of-range handle at the seam is INTERNAL-ERROR-shaped (a
    // handle can only be minted with a live just-pushed index; user code
    // cannot spell one).
    #[test]
    fn stale_handle_is_an_internal_error_at_the_seam() {
        crate::compiler::comptime_builtins::clear_comptime_hook_templates();
        let program = shape_ast::parse_program("fn victim(a: int) -> int { return a }")
            .expect("fixture parses");
        let def = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func.clone()),
                _ => None,
            })
            .expect("fixture has the victim fn");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        let site = ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:hookann:post"),
                ApplicationId::from_canonical_descriptor("application:test:0:0"),
                TargetIdentity::from_canonical_descriptor("function:victim"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&[]),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            Span::default(),
            Span::default(),
        );
        let mut staged = Vec::new();
        let err = compiler
            .apply_install_hook_template(7, "hookann", &def, &site, &mut staged)
            .expect_err("a dead index must be internal-error-shaped");
        assert!(
            err.to_string().contains("internal error"),
            "internal-error-shaped: {err}"
        );
        assert!(staged.is_empty());
        assert!(compiler.hook_install_registry.is_empty());
    }
}
